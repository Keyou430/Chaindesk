package dev.chaindesk.android

import android.app.Application
import android.net.Uri
import dev.chaindesk.android.profile.FrontNode
import dev.chaindesk.android.profile.ProfileData
import dev.chaindesk.android.profile.ProfileDownloader
import dev.chaindesk.android.profile.ProfileException
import dev.chaindesk.android.profile.ProfileRepository
import dev.chaindesk.android.profile.RuleAssets
import dev.chaindesk.android.session.SessionController
import dev.chaindesk.android.session.SessionIntent
import dev.chaindesk.core.CoreEngine
import dev.chaindesk.core.CoreFailure
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File

data class AppState(
    val profile: ProfileData = ProfileData(),
    val nodes: List<FrontNode> = emptyList(),
    val loaded: Boolean = false,
    val busy: Boolean = false,
    val errorCode: String? = null,
    val ruleVersion: String = "",
    val ruleError: Boolean = false,
    val revision: Long = 0,
)

class AppModel(private val application: Application) {
    val sessions = SessionController()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val repository = ProfileRepository(application)
    private val verifiedRepository = ProfileRepository(File(application.noBackupFilesDir, "last-verified-v1.bin"), "${application.packageName}.last-verified.v1")
    private val current = MutableStateFlow(AppState())
    val state = current.asStateFlow()

    init {
        scope.launch {
            val version = try { RuleAssets.prepare(application) } catch (_: Exception) { "" } catch (_: LinkageError) { "" }
            current.value = current.value.copy(ruleVersion = version, ruleError = version.isEmpty())
            try {
                val profile = repository.load() ?: ProfileData()
                val nodes = parseNodes(profile.subscription)
                current.value = current.value.copy(profile = profile, nodes = nodes, loaded = true)
                updateReady()
            } catch (error: Exception) {
                current.value = current.value.copy(loaded = true, errorCode = failureCode(error))
            } catch (_: LinkageError) {
                current.value = current.value.copy(loaded = true, errorCode = "CORE_LOAD_FAILED")
            }
        }
    }

    fun importFile(bytes: ByteArray) = edit {
        try { importSource(bytes, "") } finally { bytes.fill(0) }
    }

    fun importDocument(uri: Uri) = edit {
        val bytes = application.contentResolver.openInputStream(uri)?.use { input ->
            val output = java.io.ByteArrayOutputStream()
            val buffer = ByteArray(8192)
            while (true) {
                val count = input.read(buffer)
                if (count == -1) break
                if (output.size() + count > 10 * 1024 * 1024) throw CoreFailure("SUBSCRIPTION_TOO_LARGE")
                output.write(buffer, 0, count)
            }
            output.toByteArray()
        } ?: throw CoreFailure("FILE_READ_FAILED")
        try { importSource(bytes, "") } finally { bytes.fill(0) }
    }

    fun importUrl(url: String) = edit {
        val bytes = ProfileDownloader.download(url)
        try { importSource(bytes, url) } finally { bytes.fill(0) }
    }

    fun saveProfile(profile: ProfileData) = edit {
        validate(profile, current.value.nodes)
        if (!profile.rememberCredentials) verifiedRepository.forgetCredentials()
        repository.save(profile)
        current.value = current.value.copy(profile = profile, errorCode = null, revision = current.value.revision + 1)
        updateReady()
    }

    fun selectFront(id: String) = edit {
        if (current.value.nodes.none { it.id == id && it.rejection.isEmpty() } || id in current.value.profile.excludedFrontIds) throw CoreFailure("NO_VALID_FRONT")
        val profile = current.value.profile.copy(selectedFrontId = id)
        repository.save(profile)
        current.value = current.value.copy(profile = profile, errorCode = null, revision = current.value.revision + 1)
        updateReady()
    }

    fun excludeFront(id: String, excluded: Boolean) = edit {
        val old = current.value.profile
        val exclusions = if (excluded) old.excludedFrontIds + id else old.excludedFrontIds - id
        val selected = if (old.selectedFrontId in exclusions) current.value.nodes.firstOrNull { it.rejection.isEmpty() && it.id !in exclusions }?.id.orEmpty() else old.selectedFrontId
        val profile = old.copy(excludedFrontIds = exclusions, selectedFrontId = selected)
        repository.save(profile)
        current.value = current.value.copy(profile = profile, errorCode = null, revision = current.value.revision + 1)
        updateReady()
    }

    fun forgetCredentials() = edit {
        verifiedRepository.forgetCredentials()
        repository.forgetCredentials()?.let { current.value = current.value.copy(profile = it, errorCode = null, revision = current.value.revision + 1) }
        updateReady()
    }

    fun resetProfile() = edit {
        verifiedRepository.clear()
        repository.clear()
        current.value = current.value.copy(profile = ProfileData(), nodes = emptyList(), errorCode = null, revision = current.value.revision + 1)
        updateReady()
    }

    fun clearError() { current.value = current.value.copy(errorCode = null) }
    fun reportFailure(code: String) { current.value = current.value.copy(errorCode = code) }

    internal fun commitVerified(profile: ProfileData, node: FrontNode, generation: Long, nativeSessionId: Long,
        valid: () -> Boolean, activate: () -> Boolean): Boolean {
        val previous = current.value.profile
        if (profile.copy(selectedFrontId = previous.selectedFrontId) != previous ||
            node.id in previous.excludedFrontIds || current.value.nodes.none { it.id == node.id && it.rejection.isEmpty() }) return false
        fun pending() = sessions.isProbing(generation, nativeSessionId) && valid()
        if (!pending()) return false
        val previousVerified = verifiedRepository.load()
        val committed = previous.copy(selectedFrontId = node.id)
        var activated = false
        try {
            repository.save(committed)
            if (!pending()) return false
            verifiedRepository.save(committed)
            if (!pending() || !activate()) return false
            activated = true
            current.value = current.value.copy(profile = committed, errorCode = null, revision = current.value.revision + 1)
            return true
        } finally {
            if (!activated) {
                val primary = runCatching { repository.save(previous) }
                val history = runCatching { if (previousVerified == null) verifiedRepository.clear() else verifiedRepository.save(previousVerified) }
                if (primary.isFailure || history.isFailure) throw CoreFailure("PROFILE_WRITE_FAILED")
            }
        }
    }

    internal fun lastVerifiedFor(profile: ProfileData): Pair<ProfileData, FrontNode>? {
        val stored = verifiedRepository.load() ?: return null
        if (!stored.exitHost.trim().trimEnd('.').equals(profile.exitHost.trim().trimEnd('.'), ignoreCase = true) ||
            stored.exitPort != profile.exitPort || stored.preset != profile.preset || stored.expectedIpv4 != profile.expectedIpv4) return null
        val node = current.value.nodes.firstOrNull { it.id == stored.selectedFrontId && it.rejection.isEmpty() && it.id !in profile.excludedFrontIds } ?: return null
        return profile.copy(selectedFrontId = node.id) to node
    }

    fun chainInput(profile: ProfileData, node: FrontNode): JSONObject {
        if (profile.exitAuthenticationRequired && (profile.exitUsername.isEmpty() || profile.exitPassword.isEmpty())) throw CoreFailure("CREDENTIALS_REQUIRED")
        if (profile.preset == "chinaDirect" && current.value.ruleVersion.isEmpty()) throw CoreFailure("RULE_RESOURCE_UNAVAILABLE")
        return JSONObject().put("front", JSONObject(node.document)).put("preset", profile.preset)
            .put("ruleVersion", current.value.ruleVersion).put("exit", JSONObject()
                .put("host", profile.exitHost).put("port", profile.exitPort)
                .put("username", profile.exitUsername).put("password", profile.exitPassword))
    }

    fun candidates(profile: ProfileData = current.value.profile): List<FrontNode> {
        val eligible = current.value.nodes.filter { it.rejection.isEmpty() && it.id !in profile.excludedFrontIds }
        val selected = eligible.firstOrNull { it.id == profile.selectedFrontId }
        return if (profile.automaticFront) (listOfNotNull(selected) + eligible.filter { it.id != selected?.id }).take(3) else listOfNotNull(selected)
    }

    private fun importSource(bytes: ByteArray, url: String) {
        val nodes = CoreEngine.importSubscription(bytes).getJSONArray("nodes").let { array -> List(array.length()) { FrontNode.from(array.getJSONObject(it)) } }
        val valid = nodes.filter { it.rejection.isEmpty() }
        if (valid.isEmpty()) throw CoreFailure("NO_VALID_FRONT")
        val old = current.value.profile
        val selected = valid.firstOrNull { it.id == old.selectedFrontId && it.id !in old.excludedFrontIds } ?: valid.firstOrNull { it.id !in old.excludedFrontIds } ?: throw CoreFailure("NO_VALID_FRONT")
        val profile = old.copy(subscription = bytes.toString(Charsets.UTF_8), sourceUrl = url, selectedFrontId = selected.id)
        repository.save(profile)
        current.value = current.value.copy(profile = profile, nodes = nodes, errorCode = null, revision = current.value.revision + 1)
        updateReady()
    }

    private fun parseNodes(source: String): List<FrontNode> {
        if (source.isEmpty()) return emptyList()
        val nodes = CoreEngine.importSubscription(source.toByteArray(Charsets.UTF_8)).getJSONArray("nodes")
        return List(nodes.length()) { FrontNode.from(nodes.getJSONObject(it)) }
    }

    private fun validate(profile: ProfileData, nodes: List<FrontNode>) {
        val node = nodes.firstOrNull { it.id == profile.selectedFrontId && it.rejection.isEmpty() && it.id !in profile.excludedFrontIds } ?: throw CoreFailure("NO_VALID_FRONT")
        CoreEngine.compile(chainInput(profile, node))
        if (profile.expectedIpv4.isNotEmpty() && !validIpv4(profile.expectedIpv4)) throw CoreFailure("EXPECTED_IP_INVALID")
    }

    private fun updateReady() {
        val ready = runCatching { validate(current.value.profile, current.value.nodes) }.isSuccess
        sessions.submit(SessionIntent.Configure(ready))
    }

    @Synchronized private fun edit(operation: () -> Unit) {
        if (!current.value.loaded || current.value.busy) return
        if (sessions.state.value.active) { reportFailure("EDIT_REQUIRES_DISCONNECT"); return }
        current.value = current.value.copy(busy = true, errorCode = null)
        scope.launch {
            try { operation() } catch (error: Exception) { reportFailure(failureCode(error)) } catch (_: LinkageError) { reportFailure("CORE_LOAD_FAILED") }
            finally { current.value = current.value.copy(busy = false) }
        }
    }

    private fun validIpv4(value: String): Boolean {
        val parts = value.split('.')
        return parts.size == 4 && value != "0.0.0.0" && parts.all { it.isNotEmpty() && it.length <= 3 && it.all { digit -> digit in '0'..'9' } && it.toIntOrNull() in 0..255 && (it == "0" || !it.startsWith('0')) } && parts[0].toInt() !in 224..239
    }

    private fun failureCode(error: Exception): String = when (error) {
        is CoreFailure -> error.code
        is ProfileException -> error.code
        else -> "OPERATION_FAILED"
    }
}
