package dev.chaindesk.android

import dev.chaindesk.android.diagnostics.DiagnosticPreview
import dev.chaindesk.android.diagnostics.DiagnosticsExporter
import dev.chaindesk.android.profile.FrontNode
import dev.chaindesk.android.profile.ProfileData
import dev.chaindesk.android.session.SessionPhase
import dev.chaindesk.android.session.SessionState
import org.json.JSONObject
import java.security.MessageDigest

object DiagnosticsChecks {
    fun run() {
        val profile = ProfileData(
            subscription = "SECRET_SUBSCRIPTION_MARKER",
            sourceUrl = "https://secret-source.invalid/?token=SECRET_TOKEN_MARKER",
            selectedFrontId = "SECRET_SELECTED_ID",
            excludedFrontIds = setOf("SECRET_EXCLUDED_ID"),
            exitHost = "secret-exit.invalid",
            exitUsername = "SECRET_USERNAME_MARKER",
            exitPassword = "SECRET_PASSWORD_MARKER",
            rememberCredentials = true,
            expectedIpv4 = "198.51.100.27",
            automaticFront = true,
            healthMonitoring = true,
        )
        val app = AppState(
            profile = profile,
            nodes = listOf(
                node("SECRET_SELECTED_ID"),
                node("SECRET_EXCLUDED_ID"),
                node("SECRET_REJECTED_ID").copy(rejection = "SECRET_REJECTION_REASON"),
            ),
            loaded = true,
            busy = true,
            errorCode = "SECRET_ARBITRARY_UPPERCASE_CODE",
            ruleVersion = RULE_VERSION,
            revision = 17,
        )
        val session = SessionState(
            phase = SessionPhase.Connected, generation = 9, nativeSessionId = 41,
            tunCaptured = true, frontId = "SECRET_ACTIVE_FRONT_ID", ipv4 = "203.0.113.49",
            errorCode = "NETWORK_LOST",
        )
        val preview = capture(app, session)
        check(preview == capture(app, session) && current(preview, app, session)) { "Unchanged diagnostics were not deterministic/current" }
        check(preview.sha256 == sha256(preview.json) && preview.sha256.matches(Regex("[a-f0-9]{64}"))) { "Preview SHA-256 did not cover exact UTF-8 JSON bytes" }
        check(listOf("SECRET_", "secret-source.invalid", "secret-exit.invalid", "secret-node.invalid", "198.51.100.27", "203.0.113.49")
            .none { preview.json.contains(it) }) { "Diagnostics exposed private profile, endpoint, error text, or actual IP" }

        val json = JSONObject(preview.json)
        check(json.keys().asSequence().toSet() == setOf("schema", "appVersion", "coreVersion", "androidSdk", "abi", "ruleVersion", "appErrorCode", "session", "profile")) {
            "Diagnostics exported fields outside the whitelist"
        }
        check(json.getInt("schema") == 1 && json.getString("appVersion") == "0.1.0-dev.1" &&
            json.getString("coreVersion") == "v1.19.33" && json.getInt("androidSdk") == 37 &&
            json.getString("abi") == "x86_64" && json.getString("ruleVersion") == RULE_VERSION &&
            json.getString("appErrorCode") == "UNKNOWN_ERROR") { "Whitelisted metadata or error sanitization changed" }
        val exportedSession = json.getJSONObject("session")
        check(exportedSession.keys().asSequence().toSet() == setOf("phase", "generation", "tunCaptured", "observedIpAvailable", "errorCode")) {
            "Session diagnostics exported private fields"
        }
        check(exportedSession.getString("phase") == "Connected" && exportedSession.getLong("generation") == 9L &&
            exportedSession.getBoolean("tunCaptured") && exportedSession.getBoolean("observedIpAvailable") &&
            exportedSession.getString("errorCode") == "NETWORK_LOST") { "Session summary lost observed state or known error code" }
        val exportedProfile = json.getJSONObject("profile")
        check(exportedProfile.keys().asSequence().toSet() == setOf("nodeCount", "eligibleNodeCount", "preset", "automaticFront", "healthMonitoring", "rememberCredentials", "busy")) {
            "Profile diagnostics exported private fields"
        }
        check(exportedProfile.getInt("nodeCount") == 3 && exportedProfile.getInt("eligibleNodeCount") == 1 &&
            exportedProfile.getString("preset") == "allProxy" && exportedProfile.getBoolean("automaticFront") &&
            exportedProfile.getBoolean("healthMonitoring") && exportedProfile.getBoolean("rememberCredentials") &&
            exportedProfile.getBoolean("busy")) { "Diagnostics eligibility count or settings did not match the snapshot" }

        for ((changedApp, changedSession) in listOf(
            app.copy(revision = 18) to session,
            app to session.copy(nativeSessionId = 42),
            app to session.copy(frontId = "SECRET_OTHER_ACTIVE_FRONT"),
            app.copy(profile = profile.copy(selectedFrontId = "SECRET_OTHER_SELECTED_ID")) to session,
        )) {
            check(capture(changedApp, changedSession).json == preview.json) { "Private confirmation binding leaked into exported JSON" }
            check(!current(preview, changedApp, changedSession)) { "Private profile/session change retained stale confirmation" }
        }
        check(!current(preview, app, session.copy(generation = 10))) { "New session generation retained stale confirmation" }
        check(!current(preview, app.copy(busy = false), session)) { "Changed visible diagnostics retained stale confirmation" }
        check(!DiagnosticsExporter.isCurrent(preview, app, session, "0.2.0", "v1.19.33", 37, "x86_64")) { "Changed build metadata retained stale confirmation" }

        for (tampered in listOf(
            preview.copy(sha256 = "0".repeat(64)),
            preview.copy(sha256 = "malformed-hash"),
            preview.copy(json = "{}"),
            preview.copy(json = "{broken", sha256 = sha256("{broken")),
            preview.copy(json = preview.json + " ", sha256 = sha256(preview.json + " ")),
            preview.copy(binding = "tampered-binding"),
        )) {
            check(!current(tampered, app, session)) { "Tampered preview or confirmation binding was accepted" }
        }

        val privateChange = capture(app.copy(profile = profile.copy(subscription = "DIFFERENT_SECRET", exitPassword = "DIFFERENT_PASSWORD")), session.copy(ipv4 = "192.0.2.76"))
        check(privateChange.json == preview.json && privateChange.sha256 == preview.sha256) { "Raw secrets or actual IP affected exported JSON/hash" }
        val malformed = DiagnosticsExporter.preview(
            app.copy(profile = profile.copy(preset = "SECRET_PRESET"), ruleVersion = "SECRET_RULE_VERSION", errorCode = "https://secret.invalid/SECRET_ERROR"),
            session.copy(errorCode = "SECRET_SESSION_CODE"), "SECRET_VERSION", "SECRET_CORE_VERSION", 37, "SECRET_ABI",
        )
        check(!malformed.json.contains("SECRET") && !malformed.json.contains("secret.invalid")) { "Arbitrary metadata, preset, or error text escaped diagnostics sanitization" }
        val empty = JSONObject(capture(AppState(), SessionState()).json)
        check(empty.getJSONObject("profile").getInt("eligibleNodeCount") == 0 &&
            !empty.getJSONObject("session").getBoolean("observedIpAvailable") &&
            empty.getString("appErrorCode").isEmpty()) { "Empty diagnostics invented observations or errors" }
    }

    private fun node(id: String) = FrontNode(
        id, "SECRET_DISPLAY_NAME", "ss", "secret-node.invalid", 443, "",
        "{\"password\":\"SECRET_NODE_PASSWORD\",\"options\":\"SECRET_RAW_OPTIONS\"}",
    )

    private fun capture(app: AppState, session: SessionState): DiagnosticPreview =
        DiagnosticsExporter.preview(app, session, "0.1.0-dev.1", "v1.19.33", 37, "x86_64")

    private fun current(preview: DiagnosticPreview, app: AppState, session: SessionState): Boolean =
        DiagnosticsExporter.isCurrent(preview, app, session, "0.1.0-dev.1", "v1.19.33", 37, "x86_64")

    private fun sha256(value: String): String = MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8))
        .joinToString("") { byte -> (byte.toInt() and 255).toString(16).padStart(2, '0') }

    private const val RULE_VERSION = "meta-rules-dat-0123456789abcdef0123456789abcdef01234567"
}
