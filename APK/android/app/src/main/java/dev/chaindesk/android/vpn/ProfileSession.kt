package dev.chaindesk.android.vpn

import android.net.Network
import android.os.Handler
import android.os.SystemClock
import dev.chaindesk.android.profile.FrontNode
import dev.chaindesk.android.profile.ProfileData
import dev.chaindesk.android.profile.ProfileException
import dev.chaindesk.android.session.SessionIntent
import dev.chaindesk.android.session.SessionPhase
import dev.chaindesk.core.CoreEngine
import dev.chaindesk.core.CoreFailure
import dev.chaindesk.core.NativeBridge
import org.json.JSONObject
import java.net.Inet4Address
import java.util.concurrent.Executors
import java.util.concurrent.Future
import java.util.concurrent.TimeUnit

class ProfileSession(private val service: ChainVpnService, private val worker: Handler) {
    private val model get() = service.model
    private val controller get() = model.sessions
    private val resolver = Executors.newFixedThreadPool(2) { Thread(it, "chain-bootstrap").apply { isDaemon = true } }
    @Volatile private var pendingResolve: Pair<Long, Future<*>>? = null
    @Volatile private var generation = 0L

    fun cancelPending() {
        pendingResolve?.second?.cancel(true)
        worker.removeCallbacks(healthCheck)
        service.cancelNativeAsync()
    }

    fun close() { cancelPending(); resolver.shutdownNow() }

    fun connect(token: Long, requestedFront: String?) {
        generation = token
        if (!current(token)) return
        worker.removeCallbacks(healthCheck)
        var failure = "PROBE_UNAVAILABLE"
        val deadline = SystemClock.elapsedRealtime() + 30_000
        try {
            val profile = model.state.value.profile
            val candidates = if (requestedFront != null) model.state.value.nodes.filter { it.id == requestedFront && it.rejection.isEmpty() && it.id !in profile.excludedFrontIds } else model.candidates(profile)
            if (candidates.isEmpty()) throw CoreFailure("NO_VALID_FRONT")
            service.stopProductCore()
            val previous = model.lastVerifiedFor(profile)
            val network = service.productNetwork()
            service.captureProductTun(network)
            controller.submit(SessionIntent.Captured(token))
            for (node in candidates.take(3)) {
                if (!current(token)) { service.stopProductCore(); return }
                val budget = deadline - SystemClock.elapsedRealtime() - if (previous != null) 8_000 else 0
                if (budget <= 0) { failure = "PROBE_TIMEOUT"; break }
                try {
                    if (attempt(profile, node, token, minOf(budget, 12_000))) return
                } catch (error: Exception) {
                    failure = code(error)
                    service.stopProductCore()
                    if (!current(token)) return
                }
            }
            if (previous != null && current(token)) {
                val remaining = deadline - SystemClock.elapsedRealtime()
                if (remaining > 0) {
                    try {
                        if (attempt(previous.first, previous.second, token, minOf(remaining, 8_000))) return
                    } catch (_: Exception) { service.stopProductCore(); failure = "RECOVERY_FAILED" }
                }
            }
        } catch (error: Exception) {
            failure = code(error)
        } catch (_: LinkageError) {
            failure = "CORE_LOAD_FAILED"
        }
        service.stopProductCore()
        if (!service.stopping && controller.state.value.generation == token) {
            controller.submit(SessionIntent.Failed(token, failure, service.hasTun))
            service.refreshProfileNotification()
            if (!service.hasTun) service.finishWithoutCapture() else scheduleHealth()
        }
    }

    private fun attempt(profile: ProfileData, node: FrontNode, token: Long, budget: Long): Boolean {
        val deadline = SystemClock.elapsedRealtime() + budget
        controller.submit(SessionIntent.Trying(token, node.id))
        val input = model.chainInput(profile, node)
        val compiled = JSONObject(CoreEngine.compile(input).getString("config")).getJSONArray("proxies")
        val network = service.productNetwork()
        val front = resolve(network, compiled.getJSONObject(0).getString("server"), deadline, token)
        val exit = resolve(network, compiled.getJSONObject(1).getString("server"), deadline, token)
        if (!current(token)) return false
        service.captureProductTun(network)
        val prepared = JSONObject().put("input", input).put("frontIpv4", front).put("exitIpv4", exit).toString().toByteArray(Charsets.UTF_8)
        val id = try { service.startProductCore(prepared, network) } finally { prepared.fill(0) }
        if (!current(token)) { service.stopProductCore(); return false }
        controller.submit(SessionIntent.Started(token, id))
        service.refreshProfileNotification()
        val remaining = deadline - SystemClock.elapsedRealtime()
        if (remaining <= 0) throw CoreFailure("PROBE_TIMEOUT")
        val result = CoreEngine.probe(id, "exit", profile.expectedIpv4, remaining)
        if (!result.optBoolean("healthy")) throw CoreFailure("PROBE_UNAVAILABLE")
        if (!current(token) || service.nativeSessionId != id) return false
        val valid = { SystemClock.elapsedRealtime() < deadline && service.bindingIsCurrent(network, id) }
        if (!model.commitVerified(profile, node, token, id, valid) {
                controller.verifyAndActivate(token, id, result.optString("ipv4").ifEmpty { null }) {
                    valid() && NativeBridge.activate(id)
                }
            }) {
            throw CoreFailure(if (SystemClock.elapsedRealtime() >= deadline) "PROBE_TIMEOUT" else "CORE_SESSION_STALE")
        }
        service.refreshProfileNotification()
        scheduleHealth()
        return true
    }

    private fun resolve(network: Network, host: String, deadline: Long, token: Long): String {
        val future = resolver.submit<String> {
            network.getAllByName(host).filterIsInstance<Inet4Address>().firstOrNull { !it.isAnyLocalAddress && !it.isMulticastAddress }?.hostAddress ?: throw CoreFailure("BOOTSTRAP_REQUIRED")
        }
        pendingResolve = token to future
        return try {
            val remaining = minOf(3_000, deadline - SystemClock.elapsedRealtime())
            if (remaining <= 0) throw CoreFailure("PROBE_TIMEOUT")
            future.get(remaining, TimeUnit.MILLISECONDS)
        } catch (_: Exception) {
            future.cancel(true)
            throw CoreFailure("BOOTSTRAP_FAILED")
        } finally { pendingResolve = null }
    }

    fun networkChanged(previousBinding: Network?) {
        val token = generation
        val nativeId = service.nativeSessionId
        if (controller.state.value.generation != token) return
        controller.submit(SessionIntent.NetworkLost(token, nativeId))
        pendingResolve?.takeIf { it.first == token }?.second?.cancel(true)
        service.cancelNativeAsync(nativeId)
        worker.post {
            if (controller.state.value.generation != token || !service.boundTo(previousBinding) ||
                service.nativeSessionId != 0L && service.nativeSessionId != nativeId) return@post
            worker.removeCallbacks(healthCheck)
            service.stopProductCore()
            if (!service.stopping && controller.state.value.generation == token) {
                controller.submit(SessionIntent.Failed(token, "NETWORK_LOST", service.hasTun))
                service.refreshProfileNotification()
                if (service.hasTun && runCatching { service.productNetwork() }.isSuccess) {
                    val state = controller.state.value
                    val replacement = controller.submit(SessionIntent.Switch(state.frontId))
                    if (replacement.generation != token) connect(replacement.generation, if (model.state.value.profile.automaticFront) null else state.frontId)
                }
            }
        }
    }

    private fun scheduleHealth() {
        worker.removeCallbacks(healthCheck)
        if (!service.stopping && model.state.value.profile.healthMonitoring && service.hasTun && controller.state.value.phase == SessionPhase.Connected) worker.postDelayed(healthCheck, 60_000)
    }

    private val healthCheck = object : Runnable {
        override fun run() {
            if (service.stopping || !model.state.value.profile.healthMonitoring) return
            val state = controller.state.value
            if (state.phase != SessionPhase.Connected) return
            try {
                val result = CoreEngine.probe(state.nativeSessionId, "exit", model.state.value.profile.expectedIpv4, 12_000)
                controller.submit(SessionIntent.Observation(state.generation, state.nativeSessionId, result.optString("ipv4").ifEmpty { null }, result.optString("observationCode").ifEmpty { null }))
            } catch (error: Exception) {
                val failure = code(error)
                if (controller.state.value.generation != state.generation || service.stopping) return
                val outcome = controller.submit(SessionIntent.HealthFailed(state.generation, state.nativeSessionId, failure, model.state.value.profile.expectedIpv4.isNotEmpty()))
                if (outcome.generation == state.generation && outcome.phase == SessionPhase.Degraded) {
                    service.stopProductCore()
                    service.refreshProfileNotification()
                    if (failure !in setOf("IP_UNAVAILABLE", "IP_INCONSISTENT", "IP_MISMATCH") && model.state.value.profile.automaticFront) {
                        val replacement = controller.submit(SessionIntent.Switch(state.frontId))
                        if (replacement.generation != state.generation) connect(replacement.generation, null)
                    }
                }
            }
            service.refreshProfileNotification()
            scheduleHealth()
        }
    }

    private fun current(token: Long): Boolean = !service.stopping && controller.state.value.generation == token &&
        controller.state.value.phase in setOf(SessionPhase.Connecting, SessionPhase.Probing)

    private fun code(error: Exception): String = when (error) { is CoreFailure -> error.code; is ProfileException -> error.code; else -> "CONNECTION_FAILED" }
}
