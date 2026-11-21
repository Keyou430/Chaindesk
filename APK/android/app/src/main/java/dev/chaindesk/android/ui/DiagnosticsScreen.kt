package dev.chaindesk.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.chaindesk.android.AppState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
internal fun DiagnosticsScreen(
    app: AppState,
    preview: String?,
    hash: String?,
    onPreview: () -> Unit,
    onExport: () -> Unit,
    onDismissError: () -> Unit,
) {
    val context = LocalContext.current
    val version = remember(context) {
        runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: "开发版本"
    }
    var license by remember { mutableStateOf<LicenseDocument?>(null) }
    LazyColumn(
        modifier = Modifier.fillMaxSize(), contentPadding = PaddingValues(24.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        item { ScreenHeading("诊断与关于", "先生成脱敏预览，确认内容与 SHA-256 后再导出。") }
        app.errorCode?.let { code -> item { ErrorNotice(code, onDismissError) } }
        item {
            Section("脱敏诊断") {
                SupportingText("只导出白名单中的版本、会话状态、错误分类和检测摘要；不包含原始订阅、完整链接、密码、节点配置或访问记录。")
                Button(onClick = onPreview, enabled = app.loaded && !app.busy, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp)) {
                    Text(if (preview == null) "生成诊断预览" else "重新生成预览")
                }
            }
        }
        if (preview != null && hash != null) {
            item {
                Section("预览内容") {
                    Surface(color = MaterialTheme.colorScheme.surfaceVariant, shape = MaterialTheme.shapes.medium) {
                        SelectionContainer {
                            Text(preview, modifier = Modifier.fillMaxWidth().padding(16.dp), style = MaterialTheme.typography.bodyMedium, fontFamily = FontFamily.Monospace)
                        }
                    }
                }
            }
            item {
                Section("SHA-256") {
                    SelectionContainer {
                        Text(hash, style = MaterialTheme.typography.bodyMedium, fontFamily = FontFamily.Monospace)
                    }
                    SupportingText("导出以本次预览为准。状态变化后需重新预览，保存文件前请再次核对。")
                    OutlinedButton(onClick = onExport, enabled = app.loaded && !app.busy, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text("确认预览并导出")
                    }
                }
            }
        } else {
            item { SupportingText("尚未生成诊断预览。诊断文件仅在主动确认后通过系统文件选择器保存。") }
        }
        item {
            Section("后台运行") {
                SupportingText("连接期间由 Android 前台 VPN 服务运行，关闭此页面不会主动断开。请保留状态通知，通过通知或连接页断开。")
                SupportingText("若设备限制后台运行，可在系统应用设置中检查 ChainDesk 的电池与通知权限。始终开启 VPN、lockdown 与开机自动连接不在当前范围。")
            }
        }
        item {
            Section("关于 ChainDesk") {
                Text(version, style = MaterialTheme.typography.bodyLarge)
                SupportingText("独立 Android 固定出口链式代理 · 开发候选")
                SupportingText("本版本仍需 ARM64 真机、真实两跳、网络切换及协议组合验收。构建成功、模拟器检查或节点格式合法均不能替代验收。")
                SupportingText("内核：mihomo，随包固定源码与构建输入。规则：MetaCubeX/meta-rules-dat，随包校验，不在运行时联网补齐。")
                SupportingText(if (app.ruleVersion.isEmpty()) "规则版本：暂不可用" else "规则版本：${app.ruleVersion}")
            }
        }
        item {
            Section("开源许可与规则来源") {
                SupportingText("mihomo 采用 GNU GPL v3。规则数据保留各上游项目的独立许可与署名，包括 GeoLite2 / MaxMind 数据归属。")
                bundledLicenses.forEach { document ->
                    TextButton(onClick = { license = document }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text(document.title, modifier = Modifier.fillMaxWidth())
                    }
                }
            }
        }
    }
    license?.let { document ->
        val text by produceState("正在读取许可文本…", document.path) {
            value = withContext(Dispatchers.IO) {
                try {
                    context.assets.open(document.path).bufferedReader(Charsets.UTF_8).use { it.readText() }
                } catch (_: Exception) {
                    "许可文本暂不可用，请查阅交付包中的第三方许可。"
                }
            }
        }
        AlertDialog(
            onDismissRequest = { license = null }, title = { Text(document.title) },
            text = {
                SelectionContainer {
                    Column(Modifier.heightIn(max = 400.dp).verticalScroll(rememberScrollState())) {
                        Text(text, style = MaterialTheme.typography.bodyMedium)
                    }
                }
            },
            confirmButton = { TextButton(onClick = { license = null }) { Text("关闭") } },
        )
    }
}

private data class LicenseDocument(val title: String, val path: String)

private val bundledLicenses = listOf(
    LicenseDocument("规则来源与完整署名", "rules/NOTICE.md"),
    LicenseDocument("mihomo / MetaCubeX · GNU GPL v3", "rules/LICENSE-meta-rules-dat.txt"),
    LicenseDocument("Loyalsoldier / geoip · CC BY-SA 4.0", "rules/LICENSE-geoip.txt"),
    LicenseDocument("domain-list-community · MIT", "rules/LICENSE-domain-list-community.txt"),
    LicenseDocument("domain-list-custom · MIT", "rules/LICENSE-domain-list-custom.txt"),
    LicenseDocument("ios_rule_script · GNU GPL v2", "rules/LICENSE-ios-rule-script.txt"),
)
