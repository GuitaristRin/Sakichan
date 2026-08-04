package com.sakichan.se.data.repository

import android.content.Context
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive

data class ModelConfig(
    val apiBase: String,
    val apiKey: String,
    val extraParams: Map<String, JsonElement> = emptyMap()
)

private const val SENSENOVA_API_BASE = "https://token.sensenova.cn/v1/chat/completions"
private const val PREF_FILE = "sakichan_prefs"
private const val PREF_API_KEY = "api_key"

class AppConfigRepository(private val context: Context) {

    private val prefs = context.getSharedPreferences(PREF_FILE, Context.MODE_PRIVATE)

    fun getApiKey(): String = prefs.getString(PREF_API_KEY, "") ?: ""

    fun setApiKey(key: String) {
        prefs.edit().putString(PREF_API_KEY, key).apply()
    }

    fun getModelConfig(modelId: String): ModelConfig {
        val key = getApiKey()
        val extraParams: Map<String, JsonElement> = if (modelId == "deepseek-v4-flash") {
            mapOf("reasoning_effort" to JsonPrimitive("medium"))
        } else {
            emptyMap()
        }
        return ModelConfig(
            apiBase = SENSENOVA_API_BASE,
            apiKey = key,
            extraParams = extraParams
        )
    }
}
