package dev.chaindesk.android

import android.accessibilityservice.AccessibilityServiceInfo
import android.app.Instrumentation
import android.app.Activity
import android.content.Intent
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.VpnService
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.os.ParcelFileDescriptor
import android.view.accessibility.AccessibilityNodeInfo
import dev.chaindesk.android.session.SessionIntent
import dev.chaindesk.android.session.SessionPhase
import dev.chaindesk.android.vpn.ChainVpnService
import dev.chaindesk.core.NativeBridge
import org.json.JSONObject
import java.io.File

/** Controlled emulator only: the host coordinates independent-UID traffic at each checkpoint. */
internal class AppFlowChecks(private val instrumentation: Instrumentation) {
    private val context = instrumentation.targetContext
    private val model get() = (context.applicationContext as ChainDeskApplication).model
    private val checkpoint = File(context.cacheDir, "app-flow-state.json")
    private val command = File(context.cacheDir, "app-flow-command")

    fun runEarlyCancellation() {
        check(Build.HARDWARE == "ranchu") { "Dedicated emulator required" }
        instrumentation.waitForIdleSync()
        await("profile initialization") { model.state.value.loaded && !model.state.value.busy }
        check(model.state.value.profile.subscription.isEmpty() && !model.sessions.state.value.active) { "Empty test profile required" }
        val activity = instrumentation.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        try {
            instrumentation.runOnMainSync {
                model.sessions.submit(SessionIntent.Configure(true))
                val state = model.sessions.submit(SessionIntent.Connect("synthetic-pending-front"))
                context.startForegroundService(Intent(context, ChainVpnService::class.java).setAction(ChainVpnService.ACTION_CONNECT)
                    .putExtra(ChainVpnService.EXTRA_GENERATION, state.generation))
                model.sessions.submit(SessionIntent.Disconnect)
                context.startService(Intent(context, ChainVpnService::class.java).setAction(ChainVpnService.ACTION_STOP))
            }
            await("cancel before service handles CONNECT", 8_000) { model.sessions.state.value.phase == SessionPhase.Disconnected }
            check(!model.sessions.state.value.tunCaptured) { "Canceled initial request captured VPN" }
        } finally { instrumentation.runOnMainSync { activity.finish() } }
    }

    fun runUpgrade(data: JSONObject, mode: String, report: Bundle) {
        check(Build.HARDWARE == "ranchu" && mode in setOf("setup", "verify", "verifyAndClear")) { "Invalid owned upgrade test" }
        instrumentation.waitForIdleSync()
        await("upgrade model initialization") { model.state.value.loaded && !model.state.value.busy }
        val marker = File(context.noBackupFilesDir, "upgrade-validation.json")
        val primary = File(context.noBackupFilesDir, "profile-v1.bin")
        val history = File(context.noBackupFilesDir, "last-verified-v1.bin")
        fun hash(file: File) = java.security.MessageDigest.getInstance("SHA-256").digest(file.readBytes()).joinToString("") { "%02x".format(it) }
        if (mode == "setup") check(model.state.value.profile.subscription.isEmpty() && !model.sessions.state.value.active && !marker.exists()) { "Empty owned upgrade profile required" }
        else {
            check(marker.exists() && !model.sessions.state.value.active) { "Owned upgrade marker required" }
            val saved = JSONObject(marker.readText())
            check(hash(primary) == saved.getString("primary") && hash(history) == saved.getString("history")) { "Encrypted profile bytes changed across installation" }
            await("retained configuration validation") { model.sessions.state.value.phase == SessionPhase.Ready || model.state.value.errorCode != null }
            check(model.state.value.errorCode == null && model.sessions.state.value.phase == SessionPhase.Ready) {
                "Retained profile validation: ${model.sessions.state.value.phase}/${model.state.value.errorCode}"
            }
        }
        val activity = instrumentation.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        try {
            authorize(activity)
            if (mode == "setup") {
                val node = JSONObject(data.getJSONObject("frontNode").toString()).put("name", "升级验证前置 ✅")
                edit { model.importFile(JSONObject().put("proxies", org.json.JSONArray().put(node)).toString().toByteArray()) }
                val exit = data.getJSONObject("fixture").getJSONObject("exit")
                edit { model.saveProfile(model.state.value.profile.copy(exitHost = exit.getString("server"), exitPort = exit.getInt("port"),
                    exitUsername = exit.getString("username"), exitPassword = exit.getString("password"), rememberCredentials = true, exitAuthenticationRequired = true)) }
            }
            connect(model.state.value.profile.selectedFrontId)
            await("retained profile chain verification", 40_000) { model.sessions.state.value.phase in setOf(SessionPhase.Connected, SessionPhase.Degraded, SessionPhase.RecoveryRequired) }
            check(model.sessions.state.value.phase == SessionPhase.Connected) { "Retained profile failed chain verification" }
            val connectivity = context.getSystemService(ConnectivityManager::class.java)
            await("upgrade VPN route") { connectivity.getNetworkCapabilities(connectivity.activeNetwork)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true }
            val dnsOutput = ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(
                "am instrument -w dev.chaindesk.trafficprobe.debug/dev.chaindesk.trafficprobe.DnsInstrumentation",
            )).use { it.readBytes().toString(Charsets.UTF_8) }
            check(dnsOutput.contains("success=true") && dnsOutput.contains("vpnActive=true")) { "Independent application DNS query failed: $dnsOutput" }
            report.putString("upgradeDns", "verified")
            stop()
            if (mode == "verifyAndClear") {
                edit { model.resetProfile() }
                check(marker.delete()) { "Upgrade marker cleanup failed" }
            } else marker.writeText(JSONObject().put("primary", hash(primary)).put("history", hash(history)).toString())
            @Suppress("DEPRECATION") val version = context.packageManager.getPackageInfo(context.packageName, 0).versionCode
            report.putString("upgradeVersionCode", version.toString())
            report.putString("upgradeMode", mode)
            report.putString("upgradeChain", "verified")
        } finally { runCatching { stop() }; instrumentation.runOnMainSync { activity.finish() } }
    }

    fun run(data: JSONObject, report: Bundle, lifecycleCycles: Int = 0, networkSwitches: Int = 0) {
        require(lifecycleCycles in 0..100)
        require(networkSwitches in 0..20)
        check(Build.HARDWARE == "ranchu") { "Dedicated emulator required" }
        instrumentation.waitForIdleSync()
        await("profile initialization") { model.state.value.loaded && !model.state.value.busy }
        check(model.state.value.profile.subscription.isEmpty() && !model.sessions.state.value.active) { "Empty owned test profile required" }
        checkpoint.delete()
        command.delete()
        val fixture = data.getJSONObject("fixture")
        val exit = fixture.getJSONObject("exit")
        val activity = instrumentation.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        authorize(activity)
        val front = JSONObject(data.getJSONObject("frontNode").toString()).put("name", "受控前置 ✅")
        val wrong = JSONObject(front.toString()).put("name", "错误认证前置")
        if (front.getString("type") == "vmess") wrong.put("uuid", "00000000-0000-4000-8000-000000000001")
        else wrong.put("password", "deliberately-wrong-product-front")
        val yaml = JSONObject().put("proxies", org.json.JSONArray().put(front).put(wrong)).toString().toByteArray(Charsets.UTF_8)
        try {
            edit { model.importFile(yaml) }
            val nodes = model.state.value.nodes
            check(nodes.size == 2 && nodes.all { it.rejection.isEmpty() }) { "Controlled import failed" }
            val good = nodes.first().id
            val bad = nodes.last().id
            val profile = model.state.value.profile.copy(exitHost = exit.getString("server"), exitPort = exit.getInt("port"),
                exitUsername = exit.getString("username"), exitPassword = exit.getString("password"),
                exitAuthenticationRequired = true, rememberCredentials = true)
            edit { model.saveProfile(profile) }
            check(model.sessions.state.value.phase == SessionPhase.Ready) { "Profile not ready" }
            connect(good)
            await("paused product probe") { model.sessions.state.value.phase == SessionPhase.Probing }
            publish("probing")
            waitCommand("releaseHealth")
            await("verified product connection", 40_000) { model.sessions.state.value.phase in setOf(SessionPhase.Connected, SessionPhase.Degraded, SessionPhase.RecoveryRequired) }
            check(model.sessions.state.value.phase == SessionPhase.Connected) { "Product connect: ${model.sessions.state.value.errorCode}" }
            val initialId = model.sessions.state.value.nativeSessionId
            publish("connected")
            waitCommand("switchBad")
            connect(bad)
            await("rollback to verified front", 40_000) { model.sessions.state.value.phase in setOf(SessionPhase.Connected, SessionPhase.Degraded) }
            check(model.sessions.state.value.phase == SessionPhase.Connected && model.sessions.state.value.frontId == good &&
                model.state.value.profile.selectedFrontId == good && model.sessions.state.value.nativeSessionId != initialId) { "Verified rollback failed" }
            publish("rollbackConnected")
            waitCommand("cancel")
            connect(bad)
            await("cancelable native probe") { model.sessions.state.value.phase == SessionPhase.Probing }
            stop()
            val canceledGeneration = model.sessions.state.value.generation
            SystemClock.sleep(300)
            check(model.sessions.state.value.phase == SessionPhase.Disconnected && model.sessions.state.value.generation == canceledGeneration) { "Late probe revived canceled connection" }
            edit { model.saveProfile(profile.copy(exitPassword = "deliberately-wrong-product-exit")) }
            connect(good)
            await("wrong exit rejection", 40_000) { model.sessions.state.value.phase == SessionPhase.Degraded }
            check(model.sessions.state.value.tunCaptured && model.sessions.state.value.nativeSessionId == 0L) { "Wrong exit did not retain blocked TUN" }
            publish("wrongExitBlocked")
            waitCommand("stop")
            stop()
            if (lifecycleCycles > 0) {
                edit { model.saveProfile(profile) }
                val samples = org.json.JSONArray()
                repeat(lifecycleCycles) { cycle ->
                    connect(good)
                    await("lifecycle connection ${cycle + 1}", 40_000) {
                        model.sessions.state.value.phase in setOf(SessionPhase.Connected, SessionPhase.Degraded, SessionPhase.RecoveryRequired)
                    }
                    check(model.sessions.state.value.phase == SessionPhase.Connected) { "Lifecycle ${cycle + 1}: ${model.sessions.state.value.errorCode}" }
                    stop()
                    samples.put(resourceSample().put("cycle", cycle + 1))
                    checkpoint.writeText(JSONObject().put("checkpoint", "lifecycle").put("completed", cycle + 1).put("samples", samples).toString())
                }
                report.putString("lifecycleCycles", lifecycleCycles.toString())
                report.putString("lifecycleSamples", samples.toString())
                SystemClock.sleep(10_000)
                report.putString("idleResources", resourceSample().toString())
            }
            if (networkSwitches > 0) {
                edit { model.saveProfile(profile.copy(healthMonitoring = false)) }
                connect(good)
                await("initial network test connection", 40_000) { model.sessions.state.value.phase == SessionPhase.Connected }
                val connectivity = context.getSystemService(ConnectivityManager::class.java)
                await("VPN route for network test") { connectivity.getNetworkCapabilities(connectivity.activeNetwork)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true }
                val capturedVpn = connectivity.activeNetwork
                check(connectivity.getNetworkCapabilities(connectivity.activeNetwork)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true) { "Wi-Fi baseline required" }
                val switches = org.json.JSONArray()
                try {
                    repeat(networkSwitches) { index ->
                        val before = model.sessions.state.value
                        val wifi = index % 2 != 0
                        shell("svc wifi ${if (wifi) "enable" else "disable"}")
                        val transport = if (wifi) NetworkCapabilities.TRANSPORT_WIFI else NetworkCapabilities.TRANSPORT_CELLULAR
                        await("physical network recovery ${index + 1}", 50_000) {
                            val state = model.sessions.state.value
                            state.phase == SessionPhase.Connected && state.generation > before.generation && state.nativeSessionId != before.nativeSessionId &&
                                connectivity.getNetworkCapabilities(connectivity.activeNetwork)?.hasTransport(transport) == true
                        }
                        check(model.sessions.state.value.tunCaptured) { "Network recovery released captured TUN" }
                        check(connectivity.activeNetwork == capturedVpn) { "Network recovery replaced the original VPN network" }
                        switches.put(JSONObject().put("switch", index + 1).put("wifi", wifi).put("generation", model.sessions.state.value.generation))
                        checkpoint.writeText(JSONObject().put("checkpoint", "networkSwitches").put("completed", index + 1).toString())
                    }
                } finally { shell("svc wifi enable"); shell("svc data enable") }
                report.putString("networkSwitches", switches.toString())
                publish("networkRecovered")
                waitCommand("finishNetworks")
                stop()
            }
            report.putString("productFlow", "passed")
            report.putString("cancelGeneration", canceledGeneration.toString())
            publish("finished")
        } finally {
            runCatching { stop() }
            edit { model.resetProfile() }
            check(model.state.value.profile.subscription.isEmpty()) { "Synthetic profile cleanup failed" }
            instrumentation.runOnMainSync { activity.finish() }
        }
    }

    private fun connect(frontId: String) {
        val intent = if (model.sessions.state.value.tunCaptured) SessionIntent.Switch(frontId) else SessionIntent.Connect(frontId)
        val state = model.sessions.submit(intent)
        check(state.phase == SessionPhase.Connecting) { "Connect intent rejected" }
        context.startForegroundService(Intent(context, ChainVpnService::class.java).setAction(ChainVpnService.ACTION_CONNECT)
            .putExtra(ChainVpnService.EXTRA_GENERATION, state.generation).putExtra(ChainVpnService.EXTRA_FRONT_ID, frontId))
    }

    private fun stop() {
        if (!model.sessions.state.value.active) return
        model.sessions.submit(SessionIntent.Disconnect)
        context.startService(Intent(context, ChainVpnService::class.java).setAction(ChainVpnService.ACTION_STOP))
        await("product stop", 10_000) { model.sessions.state.value.phase == SessionPhase.Disconnected }
        await("VPN route removal") {
            val connectivity = context.getSystemService(ConnectivityManager::class.java)
            connectivity.getNetworkCapabilities(connectivity.activeNetwork)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) != true
        }
        check(JSONObject(NativeBridge.selfCheck()).optString("errorCode") != "CORE_SESSION_ACTIVE") { "Native session survived stop" }
    }

    private fun edit(operation: () -> Unit) {
        operation()
        await("profile transaction") { !model.state.value.busy }
        check(model.state.value.errorCode == null) { "Profile transaction: ${model.state.value.errorCode}" }
    }

    private fun publish(phase: String) {
        val state = model.sessions.state.value
        checkpoint.writeText(JSONObject().put("checkpoint", phase).put("phase", state.phase.name).put("generation", state.generation)
            .put("nativeSessionId", state.nativeSessionId).put("tunCaptured", state.tunCaptured).put("uid", android.os.Process.myUid()).toString())
    }

    private fun resourceSample(): JSONObject {
        val tasks = File("/proc/self/task").listFiles()!!
        val names = tasks.mapNotNull { runCatching { File(it, "comm").readText().trim() }.getOrNull() }.groupingBy { it }.eachCount()
        return JSONObject().put("fds", File("/proc/self/fd").list()!!.size).put("threads", tasks.size)
            .put("goroutines", JSONObject(NativeBridge.selfCheck()).getInt("goroutines")).put("threadNames", JSONObject(names))
    }

    private fun authorize(activity: Activity) {
        val authorization = VpnService.prepare(context) ?: return
        instrumentation.uiAutomation.serviceInfo = instrumentation.uiAutomation.serviceInfo.apply {
            flags = flags or AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
        }
        instrumentation.runOnMainSync { activity.startActivityForResult(authorization, 1000) }
        await("VPN authorization dialog") {
            instrumentation.uiAutomation.rootInActiveWindow?.findAccessibilityNodeInfosByViewId("android:id/button1")
                ?.firstOrNull()?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true
        }
        await("VPN authorization") { VpnService.prepare(context) == null }
    }

    private fun shell(command: String) {
        ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command)).use { it.readBytes() }
    }

    private fun waitCommand(expected: String) {
        await("host checkpoint $expected", 120_000) { command.exists() && command.readText().trim() == expected }
        check(command.delete()) { "Host command cleanup failed" }
    }

    private fun await(label: String, timeout: Long = 15_000, condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + timeout
        while (!condition()) {
            check(SystemClock.elapsedRealtime() < deadline) { "Timed out: $label" }
            SystemClock.sleep(20)
        }
    }
}
