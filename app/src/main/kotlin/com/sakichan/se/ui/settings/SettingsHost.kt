package com.sakichan.se.ui.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.sakichan.se.data.repository.AppConfigRepository
import org.koin.compose.koinInject

@Composable
fun SettingsHost(onBack: () -> Unit) {
    val config: AppConfigRepository = koinInject()

    var state by remember {
        mutableStateOf(
            SettingsUiState(
                apiKey = config.getApiKey(),
                modelId = config.getModelId(),
            )
        )
    }

    SettingsScreen(
        state = state,
        onBack = onBack,
        onSave = { apiKey, modelId ->
            config.setApiKey(apiKey)
            config.setModelId(modelId)
            state = state.copy(apiKey = apiKey, modelId = modelId, statusText = "已保存")
        },
    )
}
