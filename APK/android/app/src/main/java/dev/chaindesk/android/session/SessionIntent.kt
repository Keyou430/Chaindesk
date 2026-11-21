package dev.chaindesk.android.session

sealed interface SessionIntent {
    data class Configure(val ready: Boolean) : SessionIntent
    data class Connect(val frontId: String) : SessionIntent
    data class Switch(val frontId: String) : SessionIntent
    data object Disconnect : SessionIntent
    data class Started(val generation: Long, val nativeSessionId: Long) : SessionIntent
    data class Captured(val generation: Long) : SessionIntent
    data class Trying(val generation: Long, val frontId: String) : SessionIntent
    data class Verified(val generation: Long, val ipv4: String?, val nativeSessionId: Long) : SessionIntent
    data class Observation(val generation: Long, val nativeSessionId: Long, val ipv4: String?, val code: String?) : SessionIntent
    data class HealthFailed(val generation: Long, val nativeSessionId: Long, val code: String, val identityRequired: Boolean) : SessionIntent
    data class Failed(val generation: Long, val code: String, val captured: Boolean) : SessionIntent
    data class NetworkLost(val generation: Long, val nativeSessionId: Long? = null) : SessionIntent
    data class Stopped(val generation: Long) : SessionIntent
}
