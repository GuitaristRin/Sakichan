package com.sakichan.se.ui.connection

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.sakichan.se.core.model.Machine
import io.github.takahashirinta.kanesumi.controls.MetroButton
import io.github.takahashirinta.kanesumi.controls.MetroIconButton
import io.github.takahashirinta.kanesumi.controls.MetroProgressIndicator
import io.github.takahashirinta.kanesumi.controls.MetroTextField
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroColors
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroTypography
import io.github.takahashirinta.kanesumi.core.theme.MetroIcon
import io.github.takahashirinta.kanesumi.core.theme.MetroText
import io.github.takahashirinta.kanesumi.structure.MetroAppBar

data class ConnectionUiState(
    val scanning: Boolean = false,
    val machines: List<Machine> = emptyList(),
    val manualUrl: String = "",
    val connecting: Boolean = false,
    val error: String? = null,
)

@Composable
fun ConnectionScreen(
    state: ConnectionUiState,
    onRefresh: () -> Unit,
    onSelectMachine: (Machine) -> Unit,
    onManualUrlChange: (String) -> Unit,
    onConnectManual: () -> Unit,
) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current

    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .imePadding()
            .background(colors.background),
    ) {
        item {
            MetroAppBar(
                title = "连接 opencode",
                actions = {
                    MetroIconButton(onClick = onRefresh) {
                        MetroIcon(
                            imageVector = Icons.Filled.Refresh,
                            contentDescription = "重新扫描",
                            tint = colors.onSurfaceVariant,
                            sizeDp = 22.dp,
                        )
                    }
                },
            )
        }

        if (state.scanning && state.machines.isEmpty()) {
            item { ScanningRow() }
        }

        item {
            MetroText(
                text = "附近的机器",
                color = colors.onSurfaceVariant,
                style = typography.caption,
                modifier = Modifier.padding(start = 16.dp, top = 8.dp, bottom = 4.dp),
            )
        }

        if (state.machines.isEmpty() && !state.scanning) {
            item { EmptyHint() }
        }

        items(state.machines, key = { it.id }) { machine ->
            MachineRow(machine = machine, onClick = { onSelectMachine(machine) })
        }

        item {
            Spacer(Modifier.height(20.dp))
            MetroText(
                text = "手动输入地址",
                color = colors.onSurfaceVariant,
                style = typography.caption,
                modifier = Modifier.padding(start = 16.dp, bottom = 4.dp),
            )
            Column(modifier = Modifier.padding(horizontal = 16.dp)) {
                MetroTextField(
                    value = state.manualUrl,
                    onValueChange = onManualUrlChange,
                    modifier = Modifier.fillMaxWidth(),
                    placeholder = "http://192.168.1.100:4096",
                    singleLine = true,
                )
                Spacer(Modifier.height(8.dp))
                MetroButton(
                    text = if (state.connecting) "连接中…" else "连接",
                    onClick = onConnectManual,
                    modifier = Modifier.fillMaxWidth(),
                    enabled = !state.connecting && state.manualUrl.isNotBlank(),
                )
            }
        }

        state.error?.let {
            item {
                MetroText(
                    text = it,
                    color = androidx.compose.ui.graphics.Color(0xFFFFB4B4),
                    style = typography.caption,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }
        }
    }
}

@Composable
private fun ScanningRow() {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        MetroProgressIndicator(sizeDp = 18.dp, strokeDp = 2.dp)
        Spacer(Modifier.size(12.dp))
        MetroText(text = "正在扫描局域网…", color = colors.onSurfaceVariant, style = typography.caption)
    }
}

@Composable
private fun EmptyHint() {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
        MetroText(
            text = "未发现 opencode 机器",
            color = colors.onSurface,
            style = typography.body,
        )
        MetroText(
            text = "确认 PC 上已运行:opencode serve --hostname 0.0.0.0 --port 4096 --mdns",
            color = colors.onSurfaceVariant,
            style = typography.caption,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}

@Composable
private fun MachineRow(machine: Machine, onClick: () -> Unit) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        MetroIcon(
            imageVector = Icons.Filled.AccountCircle,
            contentDescription = null,
            tint = colors.primary,
            sizeDp = 22.dp,
        )
        Spacer(Modifier.size(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            MetroText(
                text = machine.name,
                color = colors.onSurface,
                style = typography.body,
            )
            MetroText(
                text = "${machine.host}:${machine.port}",
                color = colors.onSurfaceVariant,
                style = typography.caption,
            )
        }
        if (machine.version != null) {
            MetroText(
                text = machine.version,
                color = colors.onSurfaceVariant,
                style = typography.label,
            )
        }
    }
}
