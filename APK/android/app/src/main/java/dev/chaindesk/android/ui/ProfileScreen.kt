package dev.chaindesk.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import dev.chaindesk.android.AppState
import dev.chaindesk.android.profile.ProfileData
import dev.chaindesk.android.session.SessionPhase
import dev.chaindesk.android.session.SessionState

@Composable
internal fun ProfileScreen(
    app: AppState,
    session: SessionState,
    editor: ProfileEditor,
    onImportFile: () -> Unit,
    onImportUrl: (String) -> Unit,
    onSaveProfile: (ProfileData) -> Unit,
    onSelectFront: (String) -> Unit,
    onExcludeFront: (String, Boolean) -> Unit,
    onForgetCredentials: () -> Unit,
    onResetProfile: () -> Unit,
    onDismissError: () -> Unit,
) {
    val profile = editor.draft
    val editable = app.loaded && !app.busy && !session.active
    val canSwitch = app.loaded && !app.busy && (!session.active || session.tunCaptured && session.phase in setOf(SessionPhase.Connected, SessionPhase.Degraded, SessionPhase.RecoveryRequired))
    val selectedId = if (session.active) session.frontId else app.profile.selectedFrontId
    var showPassword by remember { mutableStateOf(false) }
    var showUrl by remember { mutableStateOf(false) }
    var confirmReset by remember { mutableStateOf(false) }
    LazyColumn(
        modifier = Modifier.fillMaxSize().imePadding(), contentPadding = PaddingValues(24.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        item {
            ScreenHeading("链路配置", if (session.active) "连接期间配置只读，可切换可用前置。请先断开再导入、保存或排除节点。" else "先导入订阅，再配置固定出口。修改后保存，连接使用已保存的配置。")
        }
        app.errorCode?.let { code -> item { ErrorNotice(code, onDismissError) } }
        editor.errorCode?.let { code -> item { ErrorNotice(code) } }
        item {
            Section("订阅") {
                SupportingText(if (app.nodes.isEmpty()) "尚未导入订阅。支持 UTF-8 Clash / Mihomo YAML，最多 10 MiB、5000 个节点。" else "已导入 ${app.nodes.size} 个去重节点，其中 ${app.nodes.count { it.rejection.isEmpty() && it.id !in app.profile.excludedFrontIds }} 个可选。更新失败保留上次内容。")
                OutlinedTextField(
                    value = editor.subscriptionUrl, onValueChange = { editor.subscriptionUrl = it },
                    label = { Text("HTTPS 订阅链接") }, singleLine = true, enabled = editable,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    visualTransformation = if (showUrl) VisualTransformation.None else PasswordVisualTransformation(),
                    trailingIcon = { TextButton(onClick = { showUrl = !showUrl }, enabled = editable) { Text(if (showUrl) "隐藏" else "显示") } },
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedButton(onClick = { onImportUrl(editor.subscriptionUrl.trim()) }, enabled = editable && editor.subscriptionUrl.trim().startsWith("https://", ignoreCase = true), modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                    Text(if (app.profile.sourceUrl.isEmpty()) "从链接导入" else "从链接更新")
                }
                OutlinedButton(onClick = onImportFile, enabled = editable, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("选择 YAML 文件") }
                SupportingText("链接可能含访问令牌，默认隐藏。导入只读取节点，不执行订阅脚本。")
            }
        }
        item {
            Section("固定 SOCKS5 出口") {
                OutlinedTextField(
                    value = profile.exitHost, onValueChange = { editor.update(profile.copy(exitHost = it)) },
                    label = { Text("出口 IPv4 或域名") }, supportingText = { Text("填写主机名，不含协议、路径或端口") },
                    singleLine = true, enabled = editable, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = editor.portInput, onValueChange = { editor.portInput = it }, label = { Text("出口端口") },
                    supportingText = { Text("1–65535") }, singleLine = true, enabled = editable,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), modifier = Modifier.fillMaxWidth(),
                )
                ToggleRow("用户名与密码认证", "无认证出口请关闭；关闭后保存会清空用户名和密码。", profile.exitAuthenticationRequired, editable) {
                    editor.update(profile.copy(exitAuthenticationRequired = it))
                }
                if (profile.exitAuthenticationRequired) {
                    OutlinedTextField(
                        value = profile.exitUsername, onValueChange = { editor.update(profile.copy(exitUsername = it)) },
                        label = { Text("出口用户名") }, singleLine = true, enabled = editable,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Text, autoCorrectEnabled = false),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = profile.exitPassword, onValueChange = { editor.update(profile.copy(exitPassword = it)) },
                        label = { Text("出口密码") }, singleLine = true, enabled = editable,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
                        visualTransformation = if (showPassword) VisualTransformation.None else PasswordVisualTransformation(),
                        trailingIcon = { TextButton(onClick = { showPassword = !showPassword }, enabled = editable) { Text(if (showPassword) "隐藏" else "显示") } },
                        supportingText = { Text("用户名和密码均为 1–255 个 UTF-8 字节，中文通常占多个字节") }, modifier = Modifier.fillMaxWidth(),
                    )
                    ToggleRow("记住出口凭证", "开启后加密保存；关闭后仅在本次进程内保留，重启需要重新输入。", profile.rememberCredentials, editable) {
                        editor.update(profile.copy(rememberCredentials = it))
                    }
                }
                OutlinedTextField(
                    value = profile.expectedIpv4, onValueChange = { editor.update(profile.copy(expectedIpv4 = it)) },
                    label = { Text("预期出口 IPv4（可选）") }, supportingText = { Text("留空不比较；填写后，IP 不匹配将拒绝连接") },
                    singleLine = true, enabled = editable, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
        item {
            Section("路由预设") {
                ChoiceRow("全部代理", "除局域网、回环及启动解析例外外，受代理 TCP 全部经固定出口。", profile.preset == "allProxy", editable) {
                    editor.update(profile.copy(preset = "allProxy"))
                }
                ChoiceRow("国内直连", "按随包国内域名与 IPv4 规则直连，其余受代理 TCP 经固定出口。", profile.preset == "chinaDirect", editable && !app.ruleError) {
                    editor.update(profile.copy(preset = "chinaDirect"))
                }
                if (app.ruleError) SupportingText("随包规则不可用，国内直连暂不可选。")
                SupportingText("两种预设均阻断非 DNS UDP / QUIC 与 IPv6。VPN 进程终止后不提供系统级断网保护。")
            }
        }
        item {
            Section("连接与恢复") {
                ToggleRow("自动尝试前置", "从当前前置开始，最多尝试 3 个未排除候选，总预算 30 秒；固定出口不变。", profile.automaticFront, editable) {
                    editor.update(profile.copy(automaticFront = it))
                }
                ToggleRow("整链健康监测", "仅在活动 VPN 内定期检测；连续 3 轮整链失败才降级，IP 观察失败不更换出口。", profile.healthMonitoring, editable) {
                    editor.update(profile.copy(healthMonitoring = it))
                }
                Button(onClick = { editor.validatedProfile()?.let(onSaveProfile) }, enabled = editable && app.nodes.isNotEmpty(), modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp)) {
                    Text("保存配置")
                }
            }
        }
        item {
            Section("前置节点") {
                SupportingText("相同名称通过稳定 ID 后缀区分。排除的节点不参与手动连接与自动选择；拒绝原因只显示分类。")
                if (app.nodes.isEmpty()) SupportingText("导入有效订阅后，节点会显示在这里。")
            }
        }
        items(app.nodes, key = { it.id }) { node ->
            FrontNodeRow(
                node, node.id == selectedId, node.id in app.profile.excludedFrontIds,
                selectionEnabled = canSwitch, onSelect = { onSelectFront(node.id) },
                onExclude = { onExcludeFront(node.id, node.id !in app.profile.excludedFrontIds) }, exclusionEnabled = editable,
            )
        }
        item {
            Section("本地数据") {
                SupportingText("忘记出口凭证会清除用户名与密码，并保留“此出口需要认证”的标记。订阅和出口地址保留。")
                OutlinedButton(onClick = {
                    editor.update(profile.copy(
                        exitUsername = "", exitPassword = "", rememberCredentials = false,
                        exitAuthenticationRequired = profile.exitAuthenticationRequired || app.profile.exitAuthenticationRequired,
                    ))
                    onForgetCredentials()
                }, enabled = editable, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("忘记出口凭证") }
                TextButton(onClick = { confirmReset = true }, enabled = editable, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                    Text("清除本地配置", color = MaterialTheme.colorScheme.error)
                }
            }
        }
    }
    if (confirmReset) {
        AlertDialog(
            onDismissRequest = { confirmReset = false }, title = { Text("清除本地配置？") },
            text = { Text("这会删除本机订阅、出口凭证、节点选择与设置，并移除对应加密密钥。清除后需要重新导入，无法撤销。") },
            dismissButton = { TextButton(onClick = { confirmReset = false }) { Text("保留配置") } },
            confirmButton = {
                TextButton(onClick = { confirmReset = false; if (editable) onResetProfile() }, enabled = editable) {
                    Text("清除配置", color = MaterialTheme.colorScheme.error)
                }
            },
        )
    }
}
