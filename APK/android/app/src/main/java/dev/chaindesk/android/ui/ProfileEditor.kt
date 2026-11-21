package dev.chaindesk.android.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import dev.chaindesk.android.profile.ProfileData
import java.nio.CharBuffer
import java.nio.charset.CodingErrorAction

class ProfileEditor : ViewModel() {
    var draft by mutableStateOf(ProfileData())
        private set
    var portInput by mutableStateOf("1080")
    var subscriptionUrl by mutableStateOf("")
    var errorCode by mutableStateOf<String?>(null)
        private set
    private var loadedRevision: Long? = null

    fun load(profile: ProfileData, revision: Long) {
        if (loadedRevision == revision) return
        loadedRevision = revision
        draft = profile
        portInput = profile.exitPort.toString()
        subscriptionUrl = profile.sourceUrl
        errorCode = null
    }

    fun update(profile: ProfileData) {
        draft = profile
        errorCode = null
    }

    fun validatedProfile(): ProfileData? {
        val port = portInput.toIntOrNull()
        val code = when {
            draft.exitHost.isBlank() -> "EXIT_ADDRESS_INVALID"
            port == null || port !in 1..65535 -> "EXIT_PORT_INVALID"
            draft.exitAuthenticationRequired &&
                (!validCredential(draft.exitUsername) || !validCredential(draft.exitPassword)) -> "EXIT_AUTH_INVALID"
            else -> null
        }
        errorCode = code
        if (code != null) return null
        return draft.copy(
            exitHost = draft.exitHost.trim(),
            exitPort = requireNotNull(port),
            exitUsername = if (draft.exitAuthenticationRequired) draft.exitUsername else "",
            exitPassword = if (draft.exitAuthenticationRequired) draft.exitPassword else "",
            rememberCredentials = draft.exitAuthenticationRequired && draft.rememberCredentials,
            expectedIpv4 = draft.expectedIpv4.trim(),
        )
    }

    private fun validCredential(value: String): Boolean {
        if (value.isEmpty() || value.length > 255) return false
        return try {
            val encoded = Charsets.UTF_8.newEncoder().onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT).encode(CharBuffer.wrap(value))
            val size = encoded.remaining()
            for (index in 0 until encoded.limit()) encoded.put(index, 0.toByte())
            size in 1..255
        } catch (_: Exception) {
            false
        }
    }
}
