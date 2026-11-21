package dev.chaindesk.android.session

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

class SessionController {
    private val current = MutableStateFlow(SessionState())
    val state = current.asStateFlow()

    fun isProbing(generation: Long, nativeSessionId: Long): Boolean = current.value.let {
        it.generation == generation && it.phase == SessionPhase.Probing && it.nativeSessionId == nativeSessionId && nativeSessionId != 0L
    }

    @Synchronized fun verifyAndActivate(generation: Long, nativeSessionId: Long, ipv4: String?, activate: () -> Boolean): Boolean {
        if (!isProbing(generation, nativeSessionId) || !activate()) return false
        val state = submit(SessionIntent.Verified(generation, ipv4, nativeSessionId))
        return state.generation == generation && state.nativeSessionId == nativeSessionId && state.phase == SessionPhase.Connected
    }

    @Synchronized fun submit(intent: SessionIntent): SessionState {
        val previous = current.value
        val next = when (intent) {
            is SessionIntent.Configure -> if (previous.active) previous else previous.copy(
                phase = if (intent.ready) SessionPhase.Ready else SessionPhase.Unconfigured,
                errorCode = null,
            )
            is SessionIntent.Connect -> if (previous.active || intent.frontId.isBlank()) previous else previous.copy(
                phase = SessionPhase.Connecting, generation = previous.generation + 1,
                frontId = intent.frontId, ipv4 = null, errorCode = null,
            )
            is SessionIntent.Switch -> if (!previous.tunCaptured || previous.phase !in setOf(SessionPhase.Connected, SessionPhase.Degraded, SessionPhase.RecoveryRequired) || intent.frontId.isBlank()) previous else previous.copy(
                phase = SessionPhase.Connecting, generation = previous.generation + 1,
                frontId = intent.frontId, ipv4 = null, errorCode = null,
            )
            SessionIntent.Disconnect -> if (previous.phase in setOf(SessionPhase.Disconnecting, SessionPhase.Disconnected)) previous else previous.copy(
                phase = SessionPhase.Disconnecting, generation = previous.generation + 1, ipv4 = null,
            )
            is SessionIntent.Started -> if (previous.generation != intent.generation || previous.phase != SessionPhase.Connecting || intent.nativeSessionId == 0L) previous else previous.copy(
                phase = SessionPhase.Probing, nativeSessionId = intent.nativeSessionId, tunCaptured = true,
            )
            is SessionIntent.Captured -> if (previous.generation != intent.generation || previous.phase in setOf(SessionPhase.Disconnecting, SessionPhase.Disconnected)) previous else previous.copy(
                tunCaptured = true, phase = if (previous.phase == SessionPhase.RecoveryRequired) SessionPhase.Degraded else previous.phase,
            )
            is SessionIntent.Trying -> if (previous.generation != intent.generation || previous.phase !in setOf(SessionPhase.Connecting, SessionPhase.Probing)) previous else previous.copy(
                phase = SessionPhase.Connecting, nativeSessionId = 0, frontId = intent.frontId, ipv4 = null, errorCode = null,
            )
            is SessionIntent.Verified -> if (previous.generation != intent.generation || previous.phase != SessionPhase.Probing || previous.nativeSessionId == 0L || previous.nativeSessionId != intent.nativeSessionId) previous else previous.copy(
                phase = SessionPhase.Connected, ipv4 = intent.ipv4, errorCode = null, healthFailures = 0,
            )
            is SessionIntent.Observation -> if (previous.generation != intent.generation || previous.nativeSessionId != intent.nativeSessionId || previous.phase != SessionPhase.Connected) previous else previous.copy(errorCode = intent.code, ipv4 = intent.ipv4, healthFailures = 0)
            is SessionIntent.HealthFailed -> if (previous.generation != intent.generation || previous.nativeSessionId != intent.nativeSessionId || previous.phase != SessionPhase.Connected) previous else {
                val identity = intent.code in setOf("IP_UNAVAILABLE", "IP_INCONSISTENT", "IP_MISMATCH")
                val failures = if (identity) 0 else previous.healthFailures + 1
                val blocked = identity && (intent.identityRequired || intent.code == "IP_MISMATCH") || !identity && failures >= 3
                previous.copy(phase = if (blocked) SessionPhase.Degraded else previous.phase,
                    nativeSessionId = if (blocked) 0 else previous.nativeSessionId,
                    ipv4 = if (identity || blocked) null else previous.ipv4, errorCode = intent.code, healthFailures = failures)
            }
            is SessionIntent.Failed -> if (previous.generation != intent.generation || previous.phase == SessionPhase.Disconnecting) previous else previous.copy(
                phase = if (intent.captured) SessionPhase.Degraded else SessionPhase.RecoveryRequired,
                nativeSessionId = 0, tunCaptured = intent.captured, errorCode = intent.code, ipv4 = null,
            )
            is SessionIntent.NetworkLost -> if (previous.generation != intent.generation || intent.nativeSessionId != null && previous.nativeSessionId != intent.nativeSessionId || !previous.active || previous.phase == SessionPhase.Disconnecting) previous else previous.copy(
                phase = if (previous.tunCaptured) SessionPhase.Degraded else SessionPhase.RecoveryRequired,
                errorCode = "NETWORK_LOST", ipv4 = null,
            )
            is SessionIntent.Stopped -> if (previous.generation != intent.generation || previous.phase != SessionPhase.Disconnecting) previous else previous.copy(
                phase = SessionPhase.Disconnected, nativeSessionId = 0, tunCaptured = false, ipv4 = null,
            )
        }
        current.value = next
        return next
    }
}
