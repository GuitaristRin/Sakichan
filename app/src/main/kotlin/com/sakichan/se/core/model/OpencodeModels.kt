package com.sakichan.se.core.model

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject

/**
 * opencode server API 数据模型。字段对齐 OpenAPI 3.1 spec(/doc),
 * 仅保留 MVP 需要的子集;服务端返回的多余字段经 Json(ignoreUnknownKeys) 静默丢弃。
 */

@Serializable
data class OcSession(
    val id: String,
    val title: String? = null,
    val parentID: String? = null,
    val projectID: String? = null,
    val directory: String? = null,
    val path: String? = null,
    val agent: String? = null,
    val time: OcSessionTime? = null,
    val cost: Double? = null,
)

@Serializable
data class OcSessionTime(
    val created: Long? = null,
    val completed: Long? = null,
)

@Serializable
data class OcSessionCreateRequest(
    val parentID: String? = null,
    val title: String? = null,
    val agent: String? = null,
)

/** opencode server 上的项目(对应一个工作目录)。GET /project 返回数组。 */
@Serializable
data class OcProject(
    val id: String,
    val name: String? = null,
    val worktree: String? = null,
    val vcs: String? = null,
    val time: OcProjectTime? = null,
)

/**
 * 项目时间戳。按真实 1.18.x schema:created / updated / initialized。
 * 旧版文档的 modified / accessed 已废弃。
 */
@Serializable
data class OcProjectTime(
    val created: Long? = null,
    val updated: Long? = null,
    val initialized: Long? = null,
)

/**
 * 一台被发现/连接的 opencode 机器。mDNS 扫描或手动输入产生。
 * baseUrl 是 `http://<ip>:<port>`;name 来自 mDNS 的 `opencode-{port}` 或用户自定义。
 * 连接后 health.version 填充,projectList 缓存该机器的项目。
 */
data class Machine(
    val id: String,            // 稳定标识:baseUrl 去尾斜杠后的哈希
    val baseUrl: String,
    val name: String,
    val host: String,
    val port: Int,
    val version: String? = null,
    val reachable: Boolean = false,
    val source: Source = Source.SCANNED,
) {
    enum class Source { SCANNED, MANUAL }
}

@Serializable
data class OcTextPartInput(
    val type: String = "text",
    val text: String,
)

/** POST /session/:id/message 与 /prompt_async 的请求体。parts 必填。 */
@Serializable
data class OcMessageRequest(
    val parts: List<OcTextPartInput>,
    val noReply: Boolean? = null,
    val agent: String? = null,
)

/** GET /session/:id/message 返回的消息:info + parts。 */
@Serializable
data class OcSessionMessage(
    val info: OcMessageInfo = OcMessageInfo(),
    val parts: List<OcMessagePart> = emptyList(),
)

@Serializable
data class OcMessageInfo(
    val id: String? = null,
    val sessionID: String? = null,
    val role: String? = null,
)

/** 消息 part:type 为 text / step-start / step-finish / reasoning / tool 等。 */
@Serializable
data class OcMessagePart(
    val id: String? = null,
    val type: String? = null,
    val text: String? = null,
    val callID: String? = null,
)

/** POST /session/:id/permissions/:permissionID 的请求体。 */
@Serializable
data class OcPermissionReply(
    val response: String,  // "once" | "always" | "reject"
)

@Serializable
data class OcForkRequest(
    val messageID: String? = null,
)

@Serializable
data class OcHealthResponse(
    val healthy: Boolean,
    val version: String,
)

/** 权限请求(来自 permission.asked 事件或 GET /session/:id/permission)。 */
@Serializable
data class OcPermissionRequest(
    val id: String,
    val sessionID: String,
    val permission: String,
    val patterns: List<String> = emptyList(),
    val metadata: JsonObject? = null,
    val always: List<String> = emptyList(),
    val tool: OcPermissionToolRef? = null,
)

@Serializable
data class OcPermissionToolRef(
    val messageID: String? = null,
    val callID: String? = null,
)

/**
 * opencode 事件流(经 /api/session/:id/event SSE 订阅)解析后的统一表示。
 * 服务端事件形态:{id, type, properties:{...}} 或直接 {type, ...payload}。
 * 解析层做防御性归一,这里只保留 UI 关心的子集,其余归 [Other]。
 */
sealed interface OcEvent {
    val sessionID: String?

    /** 文本 token 增量 */
    data class TextDelta(
        override val sessionID: String?,
        val messageID: String?,
        val textID: String?,
        val delta: String,
    ) : OcEvent

    /** 思考块 token 增量 */
    data class ReasoningDelta(
        override val sessionID: String?,
        val messageID: String?,
        val reasoningID: String?,
        val delta: String,
    ) : OcEvent

    /** 工具调用开始 */
    data class ToolCalled(
        override val sessionID: String?,
        val messageID: String?,
        val callID: String?,
        val tool: String,
        val input: JsonElement? = null,
    ) : OcEvent

    /** 工具调用成功 */
    data class ToolSuccess(
        override val sessionID: String?,
        val messageID: String?,
        val callID: String?,
        val output: String? = null,
    ) : OcEvent

    /** 工具调用失败 */
    data class ToolFailed(
        override val sessionID: String?,
        val messageID: String?,
        val callID: String?,
        val error: String,
    ) : OcEvent

    /** 一步(agent step)开始 */
    data class StepStarted(
        override val sessionID: String?,
        val messageID: String?,
        val agent: String? = null,
    ) : OcEvent

    /** 一步结束 */
    data class StepEnded(
        override val sessionID: String?,
        val messageID: String?,
        val reason: String? = null,
    ) : OcEvent

    /** 权限请求 -- 需要用户在手机上 approve/deny */
    data class PermissionAsked(
        override val sessionID: String?,
        val requestID: String,
        val permission: String,
        val patterns: List<String> = emptyList(),
        val tool: OcPermissionToolRef? = null,
    ) : OcEvent

    /** 会话空闲 -- 当前 prompt 处理完毕 */
    data class SessionIdle(override val sessionID: String?) : OcEvent

    /** 会话出错 */
    data class SessionError(override val sessionID: String?, val error: String) : OcEvent

    /** 兜底:不关心的事件 */
    data class Other(
        override val sessionID: String?,
        val type: String,
        val raw: JsonElement?,
    ) : OcEvent

    companion object {
        // v1 事件流(/event?directory=)实测类型
        const val TYPE_TEXT_DELTA = "message.part.delta"
        const val TYPE_PART_UPDATED = "message.part.updated"
        const val TYPE_SESSION_STATUS = "session.status"
        const val TYPE_SESSION_IDLE = "session.idle"
        const val TYPE_SESSION_ERROR = "session.error"
        const val TYPE_STEP_FAILED = "session.next.step.failed"

        // 旧文档/备用类型(部分 serve 可能推)
        const val TYPE_REASONING_DELTA = "session.next.reasoning.delta"
        const val TYPE_TOOL_CALLED = "session.next.tool.called"
        const val TYPE_TOOL_SUCCESS = "session.next.tool.success"
        const val TYPE_TOOL_FAILED = "session.next.tool.failed"
        const val TYPE_TOOL_PROGRESS = "session.next.tool.progress"
        const val TYPE_PERMISSION_ASKED = "permission.asked"
        const val TYPE_PERMISSION_V2_ASKED = "permission.v2.asked"
    }
}
