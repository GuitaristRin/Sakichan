package com.sakichan.se.data.repository

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive

data class ModelConfig(
    val apiBase: String,
    val apiKey: String,
    val extraParams: Map<String, JsonElement> = emptyMap()
)

private const val SENSENOVA_API_BASE = "https://token.sensenova.cn/v1/chat/completions"
private const val PREF_FILE = "sakichan_prefs"
private const val ENCRYPTED_PREF_FILE = "sakichan_secrets"
private const val PREF_API_KEY = "api_key"
private const val PREF_MODEL_ID = "secretary_model_id"
private const val DEFAULT_MODEL_ID = "deepseek-v4-flash"

/**
 * 设置存储:API key 走 EncryptedSharedPreferences,server URL / 模型 id 走普通 prefs。
 * server URL 是局域网 opencode 地址(明文无所谓),API key 是 SenseNova 凭证(必须加密)。
 */
class AppConfigRepository(private val context: Context) {

    private val prefs = context.getSharedPreferences(PREF_FILE, Context.MODE_PRIVATE)

    private val encryptedPrefs by lazy {
        val masterKey = MasterKey.Builder(context)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()
        EncryptedSharedPreferences.create(
            context,
            ENCRYPTED_PREF_FILE,
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    }

    fun getApiKey(): String = encryptedPrefs.getString(PREF_API_KEY, "") ?: ""

    fun setApiKey(key: String) {
        encryptedPrefs.edit().putString(PREF_API_KEY, key).apply()
    }

    fun getModelId(): String = prefs.getString(PREF_MODEL_ID, DEFAULT_MODEL_ID) ?: DEFAULT_MODEL_ID

    fun setModelId(modelId: String) {
        prefs.edit().putString(PREF_MODEL_ID, modelId).apply()
    }

    fun getModelConfig(modelId: String): ModelConfig {
        val key = getApiKey()
        val extraParams: Map<String, JsonElement> = if (modelId == DEFAULT_MODEL_ID) {
            mapOf("reasoning_effort" to JsonPrimitive("medium"))
        } else {
            emptyMap()
        }
        return ModelConfig(
            apiBase = SENSENOVA_API_BASE,
            apiKey = key,
            extraParams = extraParams,
        )
    }

    /** 配置是否齐全:有 API key 且 server URL 不是默认占位。 */
    fun isConfigured(): Boolean = getApiKey().isNotBlank()
}
