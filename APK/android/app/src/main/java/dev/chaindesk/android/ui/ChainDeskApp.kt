package dev.chaindesk.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp
import dev.chaindesk.android.AppModel

@Composable
fun ChainDeskApp(
    model: AppModel,
    editor: ProfileEditor,
    onImportFile: () -> Unit,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onSwitchFront: (String) -> Unit,
    diagnosticPreview: String?,
    diagnosticHash: String?,
    onPreviewDiagnostics: () -> Unit,
    onExportDiagnostics: () -> Unit,
) {
    val app by model.state.collectAsState()
    val session by model.sessions.state.collectAsState()
    var tab by rememberSaveable { mutableIntStateOf(0) }
    LaunchedEffect(app.loaded, app.revision) {
        if (app.loaded) editor.load(app.profile, app.revision)
    }
    ChainDeskTheme {
        Scaffold(
            topBar = {
                Surface(color = MaterialTheme.colorScheme.background) {
                    Column(Modifier.statusBarsPadding()) {
                        Row(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Text("ChainDesk", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                            Text("开发候选", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        if (app.busy || !app.loaded) LinearProgressIndicator(Modifier.fillMaxWidth())
                    }
                }
            },
            bottomBar = {
                NavigationBar(containerColor = MaterialTheme.colorScheme.background) {
                    listOf("连接", "配置", "诊断").forEachIndexed { index, title ->
                        NavigationBarItem(
                            selected = tab == index, onClick = { tab = index },
                            icon = { Icon(navigationIcons[index], contentDescription = null, modifier = Modifier.size(24.dp)) },
                            label = { Text(title) },
                        )
                    }
                }
            },
        ) { padding ->
            Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.TopCenter) {
                Box(Modifier.widthIn(max = 760.dp).fillMaxHeight()) {
                    when (tab) {
                        0 -> ConnectionScreen(
                            app, session, onConnect, onDisconnect,
                            onSelectFront = { if (session.active) onSwitchFront(it) else model.selectFront(it) },
                            onOpenProfile = { tab = 1 }, onDismissError = model::clearError,
                        )
                        1 -> ProfileScreen(
                            app, session, editor, onImportFile, model::importUrl, model::saveProfile,
                            onSelectFront = { if (session.active) onSwitchFront(it) else model.selectFront(it) },
                            onExcludeFront = model::excludeFront,
                            onForgetCredentials = model::forgetCredentials,
                            onResetProfile = model::resetProfile,
                            onDismissError = model::clearError,
                        )
                        else -> DiagnosticsScreen(app, diagnosticPreview, diagnosticHash, onPreviewDiagnostics, onExportDiagnostics, model::clearError)
                    }
                }
            }
        }
    }
}

private val navigationIcons = List(3) { index ->
    ImageVector.Builder(name = "navigation$index", defaultWidth = 24.dp, defaultHeight = 24.dp, viewportWidth = 24f, viewportHeight = 24f).apply {
        path(stroke = SolidColor(Color.Black), strokeLineWidth = 1.8f, strokeLineCap = StrokeCap.Round) {
            when (index) {
                0 -> {
                    moveTo(10f, 7f); lineTo(7f, 7f); curveTo(0f, 7f, 0f, 17f, 7f, 17f); lineTo(10f, 17f)
                    moveTo(14f, 7f); lineTo(17f, 7f); curveTo(24f, 7f, 24f, 17f, 17f, 17f); lineTo(14f, 17f)
                    moveTo(8f, 12f); lineTo(16f, 12f)
                }
                1 -> {
                    moveTo(4f, 6f); lineTo(20f, 6f); moveTo(4f, 12f); lineTo(20f, 12f); moveTo(4f, 18f); lineTo(20f, 18f)
                    moveTo(8f, 4f); lineTo(8f, 8f); moveTo(16f, 10f); lineTo(16f, 14f); moveTo(10f, 16f); lineTo(10f, 20f)
                }
                else -> {
                    moveTo(6f, 3f); lineTo(15f, 3f); lineTo(19f, 7f); lineTo(19f, 21f); lineTo(5f, 21f); lineTo(5f, 3f); close()
                    moveTo(15f, 3f); lineTo(15f, 7f); lineTo(19f, 7f)
                    moveTo(9f, 11f); lineTo(15f, 11f); moveTo(9f, 15f); lineTo(15f, 15f)
                }
            }
        }
    }.build()
}
