package com.sakichan.se.ui.chat

import com.sakichan.se.core.model.OcPermissionRequest

/**
 * 聊天界面的一条可见消息。区分来源:用户 / 秘书 / 系统(opencode 反馈)。
 * secretary 的回复可能含 reasoning(思考块)与正文两部分。
 */
sealed interface ChatItem {
    val id: String

    data class User(override val id: String, val text: String) : ChatItem

    data class Secretary(
        override val id: String,
        val text: String,
        val reasoning: String?,
        val streaming: Boolean,
    ) : ChatItem

    /** opencode 任务的状态行:正在执行 / 工具调用 / 完成 / 出错 */
    data class Task(
        override val id: String,
        val instruction: String,
        val status: TaskStatus,
        val detail: String? = null,
    ) : ChatItem

    data class Error(override val id: String, val text: String) : ChatItem
}

enum class TaskStatus { RUNNING, TOOL, AWAITING_PERMISSION, DONE, FAILED }

/** 待处理的权限请求,UI 弹确认条。 */
data class PendingPermission(
    val request: OcPermissionRequest,
    val taskId: String,
)

/** ChatViewModel 的整体 UI 状态。 */
data class ChatUiState(
    val items: List<ChatItem> = emptyList(),
    val isLoading: Boolean = false,
    val inputText: String = "",
    val sessionId: String? = null,
    val pendingPermission: PendingPermission? = null,
    val notConfigured: Boolean = false,
)
