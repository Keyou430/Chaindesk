package dev.chaindesk.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.chaindesk.android.AppState
import dev.chaindesk.android.session.SessionPhase
import dev.chaindesk.android.session.SessionState

@Composable
internal fun ConnectionScreen(
    app: AppState,
    session: SessionState,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onSelectFront: (String) -> Unit,
    onOpenProfile: () -> Unit,
    onDismissError: () -> Unit,
) {
    var showFronts by remember { mutableStateOf(false) }
    val selectedId = if (session.active) session.frontId else app.profile.selectedFrontId
    val front = app.nodes.firstOrNull { it.id == selectedId }
    val canSwitch = !app.busy && app.loaded && (!session.active || session.tunCaptured && session.phase in setOf(SessionPhase.Connected, SessionPhase.Degraded, SessionPhase.RecoveryRequired))
    val canConnect = app.loaded && !app.busy && session.phase != SessionPhase.Unconfigured &&
        front != null && front.rejection.isEmpty() && front.id !in app.profile.excludedFrontIds &&
        app.profile.exitHost.isNotBlank() && (!app.profile.exitAuthenticationRequired || app.profile.exitUsername.isNotEmpty() && app.profile.exitPassword.isNotEmpty())
    LazyColumn(
        modifier = Modifier.fillMaxSize(), contentPadding = PaddingValues(24.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        item {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite }) {
                SupportingText("固定出口 · 两跳 TCP")
                Text(if (app.loaded) SessionLabels.title(session) else "正在读取配置", style = MaterialTheme.typography.headlineLarge, modifier = Modifier.semantics { heading() })
                SupportingText(if (app.loaded) SessionLabels.detail(session) else "正在读取本地加密配置与随包规则。")
                if (session.phase in setOf(SessionPhase.Connecting, SessionPhase.Probing, SessionPhase.Disconnecting)) {
                    LinearProgressIndicator(Modifier.fillMaxWidth())
                }
            }
        }
        app.errorCode?.let { code -> item { ErrorNotice(code, onDismissError) } }
        session.errorCode?.takeIf { it != app.errorCode }?.let { code -> item { ErrorNotice(code) } }
        item {
            Button(
                onClick = if (session.active) onDisconnect else onConnect,
                enabled = if (session.active) session.phase != SessionPhase.Disconnecting else canConnect,
                modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp),
            ) {
                Text(when {
                    session.phase == SessionPhase.Disconnecting -> "正在断开"
                    session.phase in setOf(SessionPhase.Connecting, SessionPhase.Probing) -> "取消连接"
                    session.active -> "断开连接"
                    session.phase == SessionPhase.RecoveryRequired -> "重试连接"
                    else -> "连接并验证"
                })
            }
        }
        item {
            Surface(color = MaterialTheme.colorScheme.surfaceVariant, shape = MaterialTheme.shapes.large) {
                Column(Modifier.fillMaxWidth().padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Text("已保存的链路", style = MaterialTheme.typography.titleMedium)
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        SupportingText(if (session.active) "01  本次会话前置" else "01  所选前置")
                        Text(
                            front?.name?.ifBlank { "未命名节点" } ?: "尚未选择前置",
                            style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium,
                            maxLines = 3, overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.semantics { contentDescription = front?.name?.ifBlank { "未命名节点" } ?: "尚未选择前置" },
                        )
                        front?.let { SupportingText("${protocolLabel(it.protocol)} · ID ${it.id.takeLast(8)}") }
                    }
                    Text("↓", color = MaterialTheme.colorScheme.primary)
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        SupportingText("02  固定 SOCKS5 出口")
                        Text(exitEndpoint(app.profile.exitHost, app.profile.exitPort), style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium)
                        SupportingText(if (app.profile.exitAuthenticationRequired) "需要用户名与密码认证" else "无认证")
                    }
                    OutlinedButton(onClick = { showFronts = true }, enabled = app.nodes.isNotEmpty() && canSwitch, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text(if (session.active) "切换前置" else "选择前置")
                    }
                }
            }
        }
        item {
            Section("出口 IPv4 观察") {
                Text(session.ipv4 ?: "暂不可用", style = MaterialTheme.typography.headlineSmall)
                SupportingText(if (session.ipv4 == null) "未获取可靠的本次观察结果。HTTPS 整链检查与 IP 观察分别呈现。" else "来自本次整链的外部 IPv4 观察，不代表供应商提供独享或永久固定 IP。")
                if (app.profile.expectedIpv4.isNotEmpty()) SupportingText("预期 IPv4：${app.profile.expectedIpv4}")
            }
        }
        item {
            Section("适用范围") {
                SupportingText(if (app.profile.preset == "chinaDirect") "国内直连：按随包规则直连国内域名与 IPv4；其余受代理 TCP 经上述固定出口。" else "全部代理：除下述明确例外外，受代理 TCP 经上述固定出口。")
                SupportingText("直连例外：localhost、.local、回环地址、10/8、172.16/12、192.168/16、169.254/16，以及前置与出口的启动解析。")
                SupportingText("非 DNS UDP（含 QUIC）与 IPv6 在 VPN 接管期间阻断。进程终止或系统撤销 VPN 后不提供系统级断网保护。")
            }
        }
        item {
            SupportingText("开发候选版本。ARM64 真机、真实两跳及网络切换验收仍未完成；节点通过格式筛选不代表协议组合已通过设备验收。")
            TextButton(onClick = onOpenProfile, modifier = Modifier.heightIn(min = 48.dp)) { Text("前往配置") }
        }
    }
    if (showFronts) {
        AlertDialog(
            onDismissRequest = { showFronts = false },
            title = { Text(if (session.active) "切换前置" else "选择前置") },
            text = {
                LazyColumn(Modifier.heightIn(max = 400.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    item { SupportingText(if (session.active) "固定出口保持不变，切换后重新验证整链。" else "同名节点通过稳定 ID 后缀区分。") }
                    items(app.nodes, key = { it.id }) { node ->
                        FrontNodeRow(node, node.id == selectedId, node.id in app.profile.excludedFrontIds, canSwitch, onSelect = {
                            onSelectFront(node.id)
                            showFronts = false
                        })
                    }
                }
            },
            confirmButton = { TextButton(onClick = { showFronts = false }) { Text("关闭") } },
        )
    }
}

private fun exitEndpoint(host: String, port: Int): String =
    if (host.isBlank() || host.length > 253 || host.any { it.isWhitespace() || it.isISOControl() || it in ":/@?#[]\\" }) "尚未配置出口" else "$host:$port"
