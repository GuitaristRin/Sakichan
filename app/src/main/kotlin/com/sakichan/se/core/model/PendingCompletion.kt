package com.sakichan.se.core.model

import kotlinx.serialization.Serializable

/**
 * opencode 任务完成后的持久化记录(跨进程恢复用)。
 *
 * 场景:用户发任务后 app 被系统杀掉 -> 前台服务存活,轮询检测到任务完成 ->
 * 将结果存入 [PendingCompletionStore] -> 用户重开 app -> ViewModel 读到它,
 * 补跑对抗审查 + 总结轮,然后清除。
 *
 * 如果服务也被杀:用户重开 app 时,ViewModel 会主动 poll opencode server
 * 检查 session 状态,检测到已完成则直接取结果,无需此记录。
 */
@Serializable
data class PendingCompletion(
    val baseUrl: String,
    val opencodeSessionId: String,
    val sakichanSessionId: String,
    val instruction: String,
    val output: String,
    val failed: Boolean = false,
    val error: String? = null,
    val timestamp: Long = System.currentTimeMillis(),
)
