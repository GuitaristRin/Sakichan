package com.sakichan.se.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.takahashirinta.kanesumi.controls.MetroButton
import io.github.takahashirinta.kanesumi.controls.MetroIconButton
import io.github.takahashirinta.kanesumi.controls.MetroTextField
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroColors
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroTypography
import io.github.takahashirinta.kanesumi.core.theme.MetroIcon
import io.github.takahashirinta.kanesumi.core.theme.MetroText
import io.github.takahashirinta.kanesumi.structure.MetroAppBar

data class SettingsUiState(
    val apiKey: String,
    val modelId: String,
    val statusText: String? = null,
)

@Composable
fun SettingsScreen(
    state: SettingsUiState,
    onBack: () -> Unit,
    onSave: (apiKey: String, modelId: String) -> Unit,
) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    var apiKey by remember { mutableStateOf(state.apiKey) }
    var modelId by remember { mutableStateOf(state.modelId) }

    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .background(colors.background),
    ) {
        item {
            MetroAppBar(
                title = "设置",
                navigationIcon = {
                    MetroIconButton(onClick = onBack) {
                        MetroIcon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "返回",
                            tint = colors.onSurface,
                            sizeDp = 22.dp,
                        )
                    }
                },
            )
        }
        item {
            Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                SectionLabel("SenseNova API Key")
                MetroTextField(
                    value = apiKey,
                    onValueChange = { apiKey = it },
                    modifier = Modifier.fillMaxWidth(),
                    placeholder = "sk-...",
                    singleLine = true,
                )
                Spacer(Modifier.height(16.dp))
                SectionLabel("秘书模型 ID")
                MetroTextField(
                    value = modelId,
                    onValueChange = { modelId = it },
                    modifier = Modifier.fillMaxWidth(),
                    placeholder = "deepseek-v4-flash",
                    singleLine = true,
                )
                Spacer(Modifier.height(20.dp))
                MetroButton(
                    text = "保存",
                    onClick = { onSave(apiKey, modelId) },
                    modifier = Modifier.fillMaxWidth(),
                    leadingIcon = Icons.Filled.Check,
                )
                if (state.statusText != null) {
                    Spacer(Modifier.height(12.dp))
                    MetroText(
                        text = state.statusText,
                        color = colors.onSurfaceVariant,
                        style = typography.caption,
                    )
                }
            }
        }
    }
}

@Composable
private fun SectionLabel(text: String) {
    MetroText(
        text = text,
        color = LocalMetroColors.current.onSurfaceVariant,
        style = LocalMetroTypography.current.caption,
        modifier = Modifier.padding(bottom = 6.dp),
    )
}
