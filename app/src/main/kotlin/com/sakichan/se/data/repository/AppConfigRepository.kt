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

/**
 * 模型角色(BUILD.md §4 重构 v3)。借鉴 aider architect/editor 与 sena R1-R12 编成表:
 * 思考/总结/审查三角色可分别配模型,默认同模型但结构就位,换模型只改一处。
 *  - THINKING:思考层多轮循环,需强推理
 *  - SUMMARY:总结轮,需快与稳
 *  - REVIEW:对抗审查轮,理想用异模型防自批(ringleader creator/critic 分离)
 */
enum class ModelRole { THINKING, SUMMARY, REVIEW }

data class ModelRoster(
    val thinking: ModelConfig,
    val summary: ModelConfig,
    val review: ModelConfig,
) {
    fun forRole(role: ModelRole): ModelConfig = when (role) {
        ModelRole.THINKING -> thinking
        ModelRole.SUMMARY -> summary
        ModelRole.REVIEW -> review
    }
}

private const val SENSENOVA_API_BASE = "https://token.sensenova.cn/v1/chat/completions"
private const val PREF_FILE = "sakichan_prefs"
private const val ENCRYPTED_PREF_FILE = "sakichan_secrets"
private const val PREF_API_KEY = "api_key"
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

    fun getModelId(): String = DEFAULT_MODEL_ID

    fun getModelConfig(modelId: String): ModelConfig {
        val key = getApiKey()
        // DSV4F 硬编码。reasoning_effort: "none" 关闭思考模式--思考深度靠思考层多轮
        // 循环 + 工具收集数据体现,不靠模型自带 reasoning(避免 400 与"思考完不干活")。
        return ModelConfig(
            apiBase = SENSENOVA_API_BASE,
            apiKey = key,
            extraParams = mapOf("reasoning_effort" to JsonPrimitive("none")),
        )
    }

    /**
     * 模型编成表:三角色配置。当前默认同模型(DSV4F + reasoning_effort none),
     * 结构就位后换模型只改此处。审查轮未来可换异模型实现真正的 creator/critic 分离。
     */
    fun getRoster(): ModelRoster {
        val base = getModelConfig(DEFAULT_MODEL_ID)
        return ModelRoster(
            thinking = base,
            summary = base,
            review = base,
        )
    }

    /** 配置是否齐全:有 API key 且 server URL 不是默认占位。 */
    fun isConfigured(): Boolean = getApiKey().isNotBlank()
}
