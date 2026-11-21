package dev.chaindesk.android.diagnostics

import dev.chaindesk.android.AppState
import dev.chaindesk.android.session.SessionState
import org.json.JSONObject
import java.security.MessageDigest

data class DiagnosticPreview(val json: String, val sha256: String, internal val binding: String)

object DiagnosticsExporter {
    fun preview(app: AppState, session: SessionState, version: String, coreVersion: String, androidSdk: Int, abi: String): DiagnosticPreview {
        val json = JSONObject()
            .put("schema", 1).put("appVersion", version.takeIf(::validVersion) ?: "unknown")
            .put("coreVersion", coreVersion.takeIf(::validVersion) ?: "unknown")
            .put("androidSdk", androidSdk.coerceIn(0, 100))
            .put("abi", abi.takeIf { it in setOf("arm64-v8a", "x86_64") } ?: "unknown")
            .put("ruleVersion", app.ruleVersion.takeIf { it.matches(Regex("meta-rules-dat-[a-f0-9]{40}")) }.orEmpty())
            .put("appErrorCode", safeCode(app.errorCode))
            .put("session", JSONObject().put("phase", session.phase.name).put("generation", session.generation)
                .put("tunCaptured", session.tunCaptured).put("observedIpAvailable", !session.ipv4.isNullOrEmpty())
                .put("errorCode", safeCode(session.errorCode)))
            .put("profile", JSONObject().put("nodeCount", app.nodes.size)
                .put("eligibleNodeCount", app.nodes.count { it.rejection.isEmpty() && it.id !in app.profile.excludedFrontIds })
                .put("preset", app.profile.preset.takeIf { it in setOf("allProxy", "chinaDirect") } ?: "unknown")
                .put("automaticFront", app.profile.automaticFront).put("healthMonitoring", app.profile.healthMonitoring)
                .put("rememberCredentials", app.profile.rememberCredentials).put("busy", app.busy)).toString(2)
        val binding = JSONObject().put("revision", app.revision).put("nativeSessionId", session.nativeSessionId)
            .put("frontId", session.frontId).put("selectedFrontId", app.profile.selectedFrontId)
            .put("observation", session.ipv4.orEmpty()).put("json", json).toString()
        return DiagnosticPreview(json, hash(json), hash(binding))
    }

    fun isCurrent(preview: DiagnosticPreview, app: AppState, session: SessionState, version: String, coreVersion: String, androidSdk: Int, abi: String): Boolean =
        preview.sha256 == hash(preview.json) && preview == this.preview(app, session, version, coreVersion, androidSdk, abi)

    private fun hash(value: String): String = MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8))
        .joinToString("") { (it.toInt() and 255).toString(16).padStart(2, '0') }

    private fun validVersion(value: String): Boolean = value.length <= 64 && value.matches(Regex("v?[0-9]+\\.[0-9]+\\.[0-9]+(?:[-+][A-Za-z0-9.]+)?"))
    private fun safeCode(value: String?): String = if (value.isNullOrEmpty()) "" else if (value in knownCodes) value else "UNKNOWN_ERROR"

    private val knownCodes = setOf(
        "NO_VALID_FRONT", "EXIT_ADDRESS_INVALID", "EXIT_PORT_INVALID", "EXIT_AUTH_INVALID", "EXPECTED_IP_INVALID", "CREDENTIALS_REQUIRED",
        "NETWORK_UNAVAILABLE", "NETWORK_LOST", "NETWORK_BIND_FAILED", "BOOTSTRAP_REQUIRED", "BOOTSTRAP_FAILED",
        "VPN_DENIED", "VPN_PERMISSION_DENIED", "VPN_PERMISSION_REVOKED", "NOTIFICATION_REQUIRED", "LOCAL_NETWORK_PERMISSION_REQUIRED",
        "EDIT_REQUIRES_DISCONNECT", "SUBSCRIPTION_TOO_LARGE", "SUBSCRIPTION_UNSUPPORTED", "FILE_READ_FAILED",
        "DOWNLOAD_TOO_LARGE", "DOWNLOAD_URL_INVALID", "DOWNLOAD_AUTH_FAILED", "DOWNLOAD_TLS_FAILED", "DOWNLOAD_TIMEOUT", "DOWNLOAD_CANCELED",
        "DOWNLOAD_REDIRECT_LIMIT", "DOWNLOAD_REDIRECT_INVALID", "DOWNLOAD_ENCODING_INVALID", "DOWNLOAD_PROTOCOL_FAILED", "DOWNLOAD_HTTP_FAILED", "DOWNLOAD_FAILED",
        "RULE_RESOURCE_UNAVAILABLE", "RULE_PRESET_INVALID", "PROFILE_CORRUPT", "PROFILE_READ_FAILED", "PROFILE_KEY_MISSING", "PROFILE_KEY_INVALID",
        "PROFILE_UNSUPPORTED_VERSION", "PROFILE_SAVE_FAILED", "PROFILE_WRITE_FAILED", "PROFILE_CLEAR_FAILED", "PROFILE_INVALID", "PROFILE_TOO_LARGE", "PROFILE_BUSY",
        "PROBE_TLS_FAILED", "PROBE_TIMEOUT", "PROBE_REDIRECT", "PROBE_HTTP_STATUS", "PROBE_CANCELED", "PROBE_UNAVAILABLE", "PROBE_INPUT_INVALID", "PROBE_PATH_INVALID",
        "IP_UNAVAILABLE", "IP_INCONSISTENT", "IP_MISMATCH", "CORE_LOAD_FAILED", "CORE_CONFIG_INVALID", "CORE_START_FAILED", "CORE_STOP_FAILED",
        "CORE_SESSION_ACTIVE", "CORE_SESSION_STALE", "CORE_RESPONSE_INVALID", "CHAIN_COMPILE_FAILED", "CHAIN_INPUT_INVALID", "BRIDGE_INPUT_INVALID",
        "SERVICE_STOPPED", "CONNECTION_FAILED", "RECOVERY_FAILED", "FOREGROUND_SERVICE_UNAVAILABLE", "DIAGNOSTICS_CHANGED", "DIAGNOSTICS_EXPORT_FAILED", "OPERATION_FAILED",
    )
}
