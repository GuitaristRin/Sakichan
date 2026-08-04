package com.sakichan.se.data.repository

import android.content.Context
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import com.sakichan.se.core.model.PersistedChatSession
import com.sakichan.se.core.model.PersistedSessionMeta
import kotlinx.coroutines.flow.first
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json

private val Context.chatDataStore by preferencesDataStore(name = "chat_history")

/**
 * 本地持久化(BUILD.md Phase 2 对话管理):
 * 缓存聊天历史 + session 元数据,DataStore Preferences + kotlinx JSON。
 *
 * - 每个 session 一条记录:key `chat_<machineId>_<sessionId>`,存 [PersistedChatSession](含消息)。
 * - 每台机器一份索引:key `sessions_<machineId>`,存 [PersistedSessionMeta] 列表,
 *   供抽屉在服务器不可达时重建「机器 -> 项目 -> session」树。
 *
 * 选 DataStore 而非 Room:数据量小(聊天历史 + 元数据),JSON 序列化与现有
 * kotlinx-serialization 栈一致,无需引入 KSP 注解处理。
 */
class ChatHistoryRepository(
    private val context: Context,
    private val json: Json,
) {
    private fun chatKey(machineId: String, sessionId: String) =
        stringPreferencesKey("chat_${machineId}_$sessionId")

    private fun indexKey(machineId: String) =
        stringPreferencesKey("sessions_$machineId")

    /** 保存(覆盖)一个 session 的聊天历史,并更新该机器的索引。 */
    suspend fun saveChat(machineId: String, chat: PersistedChatSession) {
        context.chatDataStore.edit { prefs ->
            prefs[chatKey(machineId, chat.sessionId)] =
                json.encodeToString(PersistedChatSession.serializer(), chat)

            val index = decodeIndex(prefs[indexKey(machineId)])
            val meta = PersistedSessionMeta(
                sessionId = chat.sessionId,
                projectID = chat.projectID,
                title = chat.title,
                lastActiveAt = chat.lastActiveAt,
            )
            val updated = (index.filterNot { it.sessionId == chat.sessionId } + meta)
                .sortedByDescending { it.lastActiveAt }
            if (updated.isEmpty()) prefs.remove(indexKey(machineId))
            else prefs[indexKey(machineId)] =
                json.encodeToString(ListSerializer(PersistedSessionMeta.serializer()), updated)
        }
    }

    /** 只更新索引里的元数据(标题/时间),不动消息正文。抽屉拉树成功后调用。 */
    suspend fun updateIndex(machineId: String, metas: List<PersistedSessionMeta>) {
        context.chatDataStore.edit { prefs ->
            if (metas.isEmpty()) {
                prefs.remove(indexKey(machineId))
            } else {
                val merged = decodeIndex(prefs[indexKey(machineId)])
                    .map { existing ->
                        metas.find { it.sessionId == existing.sessionId } ?: existing
                    }
                val toWrite = (metas + merged)
                    .distinctBy { it.sessionId }
                    .sortedByDescending { it.lastActiveAt }
                prefs[indexKey(machineId)] =
                    json.encodeToString(ListSerializer(PersistedSessionMeta.serializer()), toWrite)
            }
        }
    }

    /** 读回一个 session 的聊天历史。不存在或损坏返回 null。 */
    suspend fun loadChat(machineId: String, sessionId: String): PersistedChatSession? {
        val prefs = context.chatDataStore.data.first()
        val raw = prefs[chatKey(machineId, sessionId)] ?: return null
        return runCatching {
            json.decodeFromString(PersistedChatSession.serializer(), raw)
        }.getOrNull()
    }

    /** 某台机器上已缓存的 session 元数据(按 lastActiveAt 降序)。 */
    suspend fun listSessions(machineId: String): List<PersistedSessionMeta> {
        val prefs = context.chatDataStore.data.first()
        return decodeIndex(prefs[indexKey(machineId)])
    }

    /** 删除一个 session 的本地缓存及其索引条目。 */
    suspend fun deleteChat(machineId: String, sessionId: String) {
        context.chatDataStore.edit { prefs ->
            prefs.remove(chatKey(machineId, sessionId))
            val updated = decodeIndex(prefs[indexKey(machineId)])
                .filterNot { it.sessionId == sessionId }
            if (updated.isEmpty()) prefs.remove(indexKey(machineId))
            else prefs[indexKey(machineId)] =
                json.encodeToString(ListSerializer(PersistedSessionMeta.serializer()), updated)
        }
    }

    private fun decodeIndex(raw: String?): List<PersistedSessionMeta> {
        if (raw.isNullOrBlank()) return emptyList()
        return runCatching {
            json.decodeFromString(ListSerializer(PersistedSessionMeta.serializer()), raw)
        }.getOrDefault(emptyList())
    }
}
