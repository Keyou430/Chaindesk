package dev.chaindesk.android.session

enum class SessionPhase {
    Unconfigured, Ready, Connecting, Probing, Connected, Degraded,
    Disconnecting, Disconnected, RecoveryRequired,
}

data class SessionState(
    val phase: SessionPhase = SessionPhase.Unconfigured,
    val generation: Long = 0,
    val nativeSessionId: Long = 0,
    val tunCaptured: Boolean = false,
    val frontId: String = "",
    val ipv4: String? = null,
    val errorCode: String? = null,
    val healthFailures: Int = 0,
) {
    val active: Boolean get() = tunCaptured || phase in setOf(SessionPhase.Connecting, SessionPhase.Probing, SessionPhase.Disconnecting)
}
