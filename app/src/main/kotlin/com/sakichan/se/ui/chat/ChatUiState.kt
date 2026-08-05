package com.sakichan.se.ui.chat

import com.sakichan.se.core.model.OcPermissionRequest
import com.sakichan.se.core.model.SessionTree

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

    /**
     * 待批准的任务提案(借鉴 sena 提案->承认闸门)。
     * 思考层产出 start_task 后不直接执行,先展示 plan 让用户批准。
     * 瞬时状态,不持久化(同 Task)。用户离开 app 时提案丢弃,回来重问即可。
     */
    data class Proposal(
        override val id: String,
        val instruction: String,
        val confirm: String,
        val plan: String,
    ) : ChatItem

    data class Error(override val id: String, val text: String) : ChatItem
}

enum class TaskStatus { RUNNING, TOOL, AWAITING_PERMISSION, DONE, FAILED }

/** 待处理的权限请求,UI 弹确认条。 */
data class PendingPermission(
    val request: OcPermissionRequest,
    val taskId: String,
)

/** opencode 运作状态(底部状态条)。null = 不显示。 */
data class OcStatus(
    val text: String,
    val lastDelta: String? = null,
)

/** ChatViewModel 的整体 UI 状态。 */
data class ChatUiState(
    val items: List<ChatItem> = emptyList(),
    val isLoading: Boolean = false,
    val inputText: String = "",
    val sessionId: String? = null,
    val pendingPermission: PendingPermission? = null,
    val notConfigured: Boolean = false,
    val drawerOpen: Boolean = false,
    val sessionTree: SessionTree? = null,
    val treeLoading: Boolean = false,
    val ocStatus: OcStatus? = null,
)
