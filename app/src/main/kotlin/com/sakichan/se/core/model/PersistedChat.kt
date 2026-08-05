package com.sakichan.se.core.model

import kotlinx.serialization.Serializable

/**
 * 本地持久化的会话:聊天历史 + 元数据。
 *
 * 以 (machineId, sessionId) 为唯一键存入 DataStore,用于重启 / 切换 session /
 * 断网后恢复聊天内容与抽屉列表。messages 直接复用 LLM 的 [Message](可序列化)。
 */
@Serializable
data class PersistedChatSession(
    val sessionId: String,
    val projectID: String? = null,
    val title: String? = null,
    val lastActiveAt: Long = 0L,
    val messages: List<Message> = emptyList(),
    val items: List<PersistedChatItem> = emptyList(),
    val opencodeSessionId: String? = null,
)

/**
 * 展示层聊天项(可序列化版)。只持久化会话对话本体(User / Secretary),
 * 任务行与错误行是瞬时执行状态,不缓存;恢复时由消息正文重建。
 */
@Serializable
data class PersistedChatItem(
    val id: String,
    val type: String,             // "user" | "secretary"
    val text: String,
    val reasoning: String? = null,
)

/**
 * 会话元数据索引(抽屉离线展示用)。
 * 不携带消息正文,只够在服务器不可达时重建「机器 -> 项目 -> session」树。
 */
@Serializable
data class PersistedSessionMeta(
    val sessionId: String,
    val projectID: String? = null,
    val title: String? = null,
    val lastActiveAt: Long = 0L,
)
