package dev.chaindesk.android

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.VpnService
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModelProvider
import dev.chaindesk.android.diagnostics.DiagnosticPreview
import dev.chaindesk.android.diagnostics.DiagnosticsExporter
import dev.chaindesk.android.session.SessionIntent
import dev.chaindesk.android.session.SessionPhase
import dev.chaindesk.android.ui.ChainDeskApp
import dev.chaindesk.android.ui.ProfileEditor
import dev.chaindesk.android.vpn.ChainVpnService
import dev.chaindesk.core.NativeBridge
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.json.JSONObject

class MainActivity : ComponentActivity() {
    private val model get() = (application as ChainDeskApplication).model
    private val io = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var preview by mutableStateOf<DiagnosticPreview?>(null)
    private var coreVersion by mutableStateOf("unknown")
    private var version = "unknown"
    private var connectRequested = false
    private var localPermissionAttempted = false
    private var requestedFront: String? = null

    private val filePicker = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> uri?.let(model::importDocument) }
    private val notificationPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) authorize() else rejectAuthorization("NOTIFICATION_REQUIRED")
    }
    private val localPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (!granted && requiresLocalTransport()) rejectAuthorization("LOCAL_NETWORK_PERMISSION_REQUIRED") else authorize()
    }
    private val vpnPermission = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        if (result.resultCode == RESULT_OK) dispatchConnection() else rejectAuthorization("VPN_PERMISSION_DENIED")
    }
    private val diagnosticFile = registerForActivityResult(ActivityResultContracts.CreateDocument("application/json")) { uri ->
        val confirmed = preview
        if (uri != null && confirmed != null) io.launch {
            if (!current(confirmed)) {
                model.reportFailure("DIAGNOSTICS_CHANGED")
                runOnUiThread { preview = null }
                return@launch
            }
            try {
                contentResolver.openOutputStream(uri, "w")?.use { it.write(confirmed.json.toByteArray(Charsets.UTF_8)) }
                    ?: throw IllegalStateException()
            } catch (_: Exception) { model.reportFailure("DIAGNOSTICS_EXPORT_FAILED") }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        connectRequested = savedInstanceState?.getBoolean("connectRequested") ?: false
        requestedFront = savedInstanceState?.getString("requestedFront")
        localPermissionAttempted = savedInstanceState?.getBoolean("localPermissionAttempted") ?: false
        version = packageManager.getPackageInfo(packageName, 0).versionName ?: "unknown"
        val editor = ViewModelProvider(this)[ProfileEditor::class.java]
        io.launch {
            val value = try { JSONObject(NativeBridge.selfCheck()).optString("coreVersion", "unknown") } catch (_: Exception) { "unknown" } catch (_: LinkageError) { "unknown" }
            runOnUiThread { coreVersion = value }
        }
        setContent {
            ChainDeskApp(
                model, editor, { filePicker.launch(arrayOf("*/*")) }, { requestConnection(null) }, ::disconnect,
                { requestConnection(it) }, preview?.json, preview?.sha256, ::previewDiagnostics, ::exportDiagnostics,
            )
        }
    }

    private fun requestConnection(front: String?) {
        val state = model.state.value
        if (!state.loaded || state.busy) { model.reportFailure("PROFILE_BUSY"); return }
        if (front != null && state.nodes.none { it.id == front && it.rejection.isEmpty() && it.id !in state.profile.excludedFrontIds }) {
            model.reportFailure("NO_VALID_FRONT"); return
        }
        if (model.sessions.state.value.phase in setOf(SessionPhase.Connecting, SessionPhase.Probing, SessionPhase.Disconnecting)) return
        requestedFront = front
        connectRequested = true
        localPermissionAttempted = false
        authorize()
    }

    private fun authorize() {
        if (!connectRequested) return
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
            return
        }
        if (Build.VERSION.SDK_INT >= 37 && !localPermissionAttempted && checkSelfPermission(Manifest.permission.ACCESS_LOCAL_NETWORK) != PackageManager.PERMISSION_GRANTED) {
            localPermissionAttempted = true
            localPermission.launch(Manifest.permission.ACCESS_LOCAL_NETWORK)
            return
        }
        val request = VpnService.prepare(this)
        if (request != null) vpnPermission.launch(request) else dispatchConnection()
    }

    private fun dispatchConnection() {
        if (!connectRequested || !model.state.value.loaded || model.state.value.busy) return
        connectRequested = false
        val previous = model.sessions.state.value
        val front = requestedFront ?: model.state.value.profile.selectedFrontId
        val next = model.sessions.submit(if (previous.tunCaptured) SessionIntent.Switch(front) else SessionIntent.Connect(front))
        if (next.generation == previous.generation) return
        val intent = Intent(this, ChainVpnService::class.java).setAction(ChainVpnService.ACTION_CONNECT)
            .putExtra(ChainVpnService.EXTRA_GENERATION, next.generation)
        requestedFront?.let { intent.putExtra(ChainVpnService.EXTRA_FRONT_ID, it) }
        try { startForegroundService(intent) } catch (_: RuntimeException) {
            model.sessions.submit(SessionIntent.Failed(next.generation, "FOREGROUND_SERVICE_UNAVAILABLE", previous.tunCaptured))
        }
    }

    private fun disconnect() {
        connectRequested = false
        model.sessions.submit(SessionIntent.Disconnect)
        startService(Intent(this, ChainVpnService::class.java).setAction(ChainVpnService.ACTION_STOP))
    }

    private fun rejectAuthorization(code: String) { connectRequested = false; model.reportFailure(code) }

    private fun requiresLocalTransport(): Boolean = listOf(
        model.state.value.nodes.firstOrNull { it.id == (requestedFront ?: model.state.value.profile.selectedFrontId) }?.server.orEmpty(),
        model.state.value.profile.exitHost,
    ).any { host ->
        val parts = host.split('.').mapNotNull(String::toIntOrNull)
        host == "localhost" || host.endsWith(".local", ignoreCase = true) || parts.size == 4 &&
            (parts[0] == 10 || parts[0] == 127 || parts[0] == 172 && parts[1] in 16..31 || parts[0] == 192 && parts[1] == 168 || parts[0] == 169 && parts[1] == 254)
    }

    private fun previewDiagnostics() { preview = DiagnosticsExporter.preview(model.state.value, model.sessions.state.value, version, coreVersion, Build.VERSION.SDK_INT, Build.SUPPORTED_ABIS.firstOrNull().orEmpty()) }
    private fun current(value: DiagnosticPreview) = DiagnosticsExporter.isCurrent(value, model.state.value, model.sessions.state.value, version, coreVersion, Build.VERSION.SDK_INT, Build.SUPPORTED_ABIS.firstOrNull().orEmpty())
    private fun exportDiagnostics() {
        val value = preview ?: return
        if (!current(value)) { preview = null; model.reportFailure("DIAGNOSTICS_CHANGED"); return }
        diagnosticFile.launch("chaindesk-diagnostics.json")
    }

    override fun onSaveInstanceState(outState: Bundle) {
        outState.putBoolean("connectRequested", connectRequested)
        outState.putString("requestedFront", requestedFront)
        outState.putBoolean("localPermissionAttempted", localPermissionAttempted)
        super.onSaveInstanceState(outState)
    }

    override fun onDestroy() { io.cancel(); super.onDestroy() }
}
