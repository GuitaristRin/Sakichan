package com.sakichan.se.data.network

import android.util.Log
import com.sakichan.se.core.model.*
import com.sakichan.se.core.error.SakichanError
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.serialization.json.*
import okhttp3.*
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.sse.EventSource
import okhttp3.sse.EventSourceListener
import okhttp3.sse.EventSources

/**
 * opencode server HTTP 客户端。直连局域网 `http://<PC-IP>:4096`。
 *
 * 核心是事件流:[events] 订阅 `/api/session/:id/event` SSE,实时拿到 agent 的
 * token 输出、工具调用、权限请求。手机是遥控器,权限经 [replyPermission] 回传。
 *
 * 设计为「无状态 + baseUrl 注入」:server URL 由 AppConfigRepository 提供,
 * 切换 PC 只需改设置,无需重建 client。baseUrl 末尾不带 `/`。
 */
class OpencodeClient(
    private val okHttpClient: OkHttpClient,
    private val json: Json,
) {

    /** 探活 + 取版本,用于设置页连通性测试。 */
    suspend fun health(baseUrl: String): OcHealthResponse {
        val resp = okHttpClient.newCall(
            Request.Builder().url("${trimSlash(baseUrl)}/global/health").get().build()
        ).await()
        return json.decodeFromString(OcHealthResponse.serializer(), resp.body!!.string())
    }

    /** 创建 session。parentID 非空时为 fork(对应 deri 分支)。 */
    suspend fun createSession(
        baseUrl: String,
        req: OcSessionCreateRequest = OcSessionCreateRequest(),
        directory: String? = null,
    ): OcSession {
        val body = json.encodeToString(OcSessionCreateRequest.serializer(), req)
            .toRequestBody(JSON)
        val url = buildString {
            append(trimSlash(baseUrl)).append("/session")
            if (!directory.isNullOrBlank()) append("?directory=").append(urlEncode(directory))
        }
        val resp = okHttpClient.newCall(
            Request.Builder().url(url).post(body).build()
        ).await()
        return json.decodeFromString(OcSession.serializer(), resp.body!!.string())
    }

    /** 列出所有 session(抽屉用)。 */
    suspend fun listSessions(baseUrl: String): List<OcSession> {
        val resp = okHttpClient.newCall(
            Request.Builder().url("${trimSlash(baseUrl)}/session").get().build()
        ).await()
        return json.decodeFromString(
            kotlinx.serialization.builtins.ListSerializer(OcSession.serializer()),
            resp.body!!.string(),
        )
    }

    suspend fun getSession(baseUrl: String, id: String): OcSession {
        val resp = okHttpClient.newCall(
            Request.Builder().url("${trimSlash(baseUrl)}/session/$id").get().build()
        ).await()
        return json.decodeFromString(OcSession.serializer(), resp.body!!.string())
    }

    /**
     * 轮询会话消息。此版本的事件流(per-session SSE)实测为空流,
     * 任务完成信号改用轮询 message:等 assistant 出现 step-finish part。
     */
    suspend fun listMessages(baseUrl: String, sessionId: String): List<OcSessionMessage> {
        val resp = okHttpClient.newCall(
            Request.Builder().url("${trimSlash(baseUrl)}/session/$sessionId/message").get().build()
        ).await()
        return json.decodeFromString(
            kotlinx.serialization.builtins.ListSerializer(OcSessionMessage.serializer()),
            resp.body!!.string(),
        )
    }

    /** 列出该机器上所有项目(工作目录)。连接后用于构建「机器->项目->session」树。 */
    suspend fun listProjects(baseUrl: String): List<OcProject> {
        val resp = okHttpClient.newCall(
            Request.Builder().url("${trimSlash(baseUrl)}/project").get().build()
        ).await()
        return json.decodeFromString(
            kotlinx.serialization.builtins.ListSerializer(OcProject.serializer()),
            resp.body!!.string(),
        )
    }

    /** 当前活跃项目(机器当前工作目录)。 */
    suspend fun currentProject(baseUrl: String): OcProject {
        val resp = okHttpClient.newCall(
            Request.Builder().url("${trimSlash(baseUrl)}/project/current").get().build()
        ).await()
        return json.decodeFromString(OcProject.serializer(), resp.body!!.string())
    }

    /**
     * 异步发消息(不等待)。返回后立即订阅 [events] 拿流式结果。
     * 用 prompt_async 而非同步 message:后者会阻塞直到 agent 跑完,失去实时性。
     */
    suspend fun promptAsync(baseUrl: String, sessionId: String, req: OcMessageRequest) {
        val body = json.encodeToString(OcMessageRequest.serializer(), req)
            .toRequestBody(JSON)
        val resp = okHttpClient.newCall(
            Request.Builder().url("${trimSlash(baseUrl)}/session/$sessionId/prompt_async").post(body).build()
        ).await()
        // 204 No Content 为正常;非 2xx 抛错
        if (!resp.isSuccessful) {
            throw SakichanError.Api(resp.code, "prompt_async failed: ${resp.body?.string()?.take(200)}")
        }
    }

    /** 权限回调:once/always/reject。手机上的 approve/deny 经此传回 opencode。 */
    suspend fun replyPermission(
        baseUrl: String,
        sessionId: String,
        permissionId: String,
        response: String,  // "once" | "always" | "reject"
    ): Boolean {
        val body = json.encodeToString(OcPermissionReply.serializer(), OcPermissionReply(response))
            .toRequestBody(JSON)
        val resp = okHttpClient.newCall(
            Request.Builder()
                .url("${trimSlash(baseUrl)}/session/$sessionId/permissions/$permissionId")
                .post(body).build()
        ).await()
        return resp.isSuccessful
    }

    /** 从某消息分叉(deri 换思路)。返回新 session。 */
    suspend fun fork(baseUrl: String, sessionId: String, messageID: String? = null): OcSession {
        val body = json.encodeToString(OcForkRequest.serializer(), OcForkRequest(messageID))
            .toRequestBody(JSON)
        val resp = okHttpClient.newCall(
            Request.Builder().url("${trimSlash(baseUrl)}/session/$sessionId/fork").post(body).build()
        ).await()
        return json.decodeFromString(OcSession.serializer(), resp.body!!.string())
    }

    /** 中止正在跑的 session。 */
    suspend fun abort(baseUrl: String, sessionId: String): Boolean {
        val resp = okHttpClient.newCall(
            Request.Builder().url("${trimSlash(baseUrl)}/session/$sessionId/abort").post(EMPTY).build()
        ).await()
        return resp.isSuccessful
    }

    /**
     * 订阅 session 事件流(SSE)。实测真实端点(v1)是 `GET /event?directory=<工作目录>`,
     * 不是 `/api/session/:id/event`(后者在 1.18.11 上是空流)。
     *
     * 事件 payload 在 `properties` 字段,核心事件:
     * - `message.part.delta`  流式文本增量(delta)
     * - `message.part.updated` part 完整快照
     * - `session.idle` / `session.error`  完成 / 出错
     * - `permission.asked`(本 serve 上未推送,权限走 `/api/permission/request` 轮询)
     *
     * 取消订阅即取消 EventSource。收到 idle/error 后关闭流。
     */
    fun events(baseUrl: String, sessionId: String, directory: String): Flow<OcEvent> = callbackFlow {
        val url = "${trimSlash(baseUrl)}/event?directory=${urlEncode(directory)}"
        val request = Request.Builder().url(url).get().build()
        val factory = EventSources.createFactory(okHttpClient)

        val listener = object : EventSourceListener() {
            override fun onEvent(source: EventSource, id: String?, type: String?, data: String) {
                if (data.isBlank()) return
                try {
                    val parsed = parseEvent(data, type)
                    // /event 是按目录的全局流,可能混入同目录其他 session 的事件
                    if (parsed.sessionID != null && parsed.sessionID != sessionId) return
                    trySend(parsed)
                    if (parsed is OcEvent.SessionIdle || parsed is OcEvent.SessionError) {
                        channel.close()
                    }
                } catch (e: Exception) {
                    Log.w("OpencodeSSE", "parse fail: ${e.message}", e)
                }
            }

            override fun onFailure(source: EventSource, t: Throwable?, response: Response?) {
                if (!channel.isClosedForSend) {
                    val msg = when {
                        t != null -> "SSE error: ${t.message}"
                        response != null -> "SSE HTTP ${response.code}${response.body?.string()?.take(200)?.let { ": $it" } ?: ""}"
                        else -> "SSE unknown error"
                    }
                    trySend(OcEvent.SessionError(sessionId, msg))
                }
                // 必须关流:否则 collect 永不结束,executeOpencodeTask 卡死
                channel.close()
            }

            override fun onClosed(source: EventSource) {
                channel.close()
            }
        }

        val source = factory.newEventSource(request, listener)
        awaitClose { source.cancel() }
    }

    /**
     * 归一化解析单条 SSE data(v1 `/event?directory=` 流)。
     *
     * 事件结构 `{id, type, properties: {...}}`,payload 在 `properties`:
     * - `message.part.delta`   delta 增量(field=text/reasoning)
     * - `message.part.updated` part 快照,按 part.type 分派(step-start/step-finish/text/tool)
     * - `session.idle` / `session.error` / `session.status`
     */
    private fun parseEvent(data: String, sseType: String?): OcEvent {
        val obj = json.parseToJsonElement(data).jsonObject
        val type = obj.str("type") ?: sseType ?: return OcEvent.Other(null, "unknown", obj)

        val props = obj["properties"]?.jsonObject ?: obj
        val sid = props.str("sessionID") ?: obj.str("sessionID")

        return when (type) {
            OcEvent.TYPE_TEXT_DELTA -> {
                val field = props.str("field")
                val delta = props.str("delta") ?: ""
                if (field == "reasoning") {
                    OcEvent.ReasoningDelta(sid, props.str("messageID"), props.str("partID"), delta)
                } else {
                    OcEvent.TextDelta(sid, props.str("messageID"), props.str("partID"), delta)
                }
            }

            OcEvent.TYPE_PART_UPDATED -> parsePartUpdated(sid, props)

            OcEvent.TYPE_SESSION_STATUS -> OcEvent.Other(sid, type, obj)

            OcEvent.TYPE_SESSION_IDLE -> OcEvent.SessionIdle(sid)

            OcEvent.TYPE_SESSION_ERROR -> OcEvent.SessionError(
                sid,
                props["error"]?.let { e ->
                    when (e) {
                        is JsonObject -> e.str("message") ?: e.toString()
                        else -> e.strOrNull() ?: e.toString()
                    }
                } ?: "session error",
            )

            OcEvent.TYPE_PERMISSION_ASKED -> OcEvent.PermissionAsked(
                sessionID = sid,
                requestID = props.str("id") ?: props.str("requestID") ?: "",
                permission = props.str("permission") ?: "unknown",
                patterns = props["patterns"]?.jsonArray?.strList() ?: emptyList(),
                tool = props["tool"]?.let { json.decodeFromJsonElement(OcPermissionToolRef.serializer(), it) },
            )

            OcEvent.TYPE_PERMISSION_V2_ASKED -> OcEvent.PermissionAsked(
                sessionID = sid,
                requestID = props.str("id") ?: "",
                permission = props.str("action") ?: "unknown",
                patterns = props["resources"]?.jsonArray?.strList() ?: emptyList(),
                tool = null,
            )

            // 旧文档备用类型:兜底映射
            OcEvent.TYPE_REASONING_DELTA -> OcEvent.ReasoningDelta(
                sid, props.str("assistantMessageID"), props.str("reasoningID"), props.str("delta") ?: "",
            )
            OcEvent.TYPE_TOOL_CALLED -> OcEvent.ToolCalled(
                sid, props.str("assistantMessageID"), props.str("callID"), props.str("tool") ?: "tool", props["input"],
            )
            OcEvent.TYPE_TOOL_SUCCESS -> OcEvent.ToolSuccess(
                sid, props.str("assistantMessageID"), props.str("callID"), props.str("output"),
            )
            OcEvent.TYPE_TOOL_FAILED, OcEvent.TYPE_STEP_FAILED -> OcEvent.ToolFailed(
                sid, props.str("assistantMessageID"), props.str("callID"),
                props["error"]?.let { err ->
                    when (err) {
                        is JsonObject -> err.str("message") ?: err.toString()
                        else -> err.strOrNull() ?: err.toString()
                    }
                } ?: type,
            )
            OcEvent.TYPE_TOOL_PROGRESS -> OcEvent.Other(sid, type, obj)

            else -> OcEvent.Other(sid, type, obj)
        }
    }

    /** message.part.updated:按 part.type 分派。 */
    private fun parsePartUpdated(sid: String?, props: JsonObject): OcEvent {
        val part = props["part"]?.jsonObject
        val partType = part?.str("type")
        val messageID = part?.str("messageID") ?: props.str("messageID")
        return when (partType) {
            "step-start" -> OcEvent.StepStarted(sid, messageID, part?.str("agent"))
            "step-finish" -> OcEvent.StepEnded(sid, messageID, part?.str("reason") ?: part?.str("reasoning"))
            "text" -> OcEvent.TextDelta(sid, messageID, part?.str("id"), part?.str("text") ?: "")
            "reasoning" -> OcEvent.ReasoningDelta(sid, messageID, part?.str("id"), part?.str("text") ?: "")
            "tool" -> OcEvent.ToolCalled(sid, messageID, part?.str("callID"), part?.str("name") ?: "tool", part)
            "error" -> OcEvent.ToolFailed(sid, messageID, part?.str("callID"), part?.str("text") ?: "tool error")
            else -> OcEvent.Other(sid, "message.part.updated", props)
        }
    }

    private fun trimSlash(url: String): String = url.trimEnd('/')

    companion object {
        private val JSON = "application/json".toMediaType()
        private val EMPTY = "".toRequestBody(JSON)
    }
}

/** URL 编码单个查询参数值(路径里的目录可能含空格等)。 */
private fun urlEncode(value: String): String =
    java.net.URLEncoder.encode(value, "UTF-8")

/** OkHttp Call 同步执行 -> suspend。非 2xx 抛 [SakichanError.Api]。 */
suspend fun Call.await(): Response {
    val resp = kotlinx.coroutines.suspendCancellableCoroutine { cont ->
        enqueue(object : Callback {
            override fun onFailure(call: Call, e: java.io.IOException) {
                if (cont.isActive) cont.resumeWith(Result.failure(e))
            }
            override fun onResponse(call: Call, response: Response) {
                if (cont.isActive) cont.resumeWith(Result.success(response))
            }
        })
        cont.invokeOnCancellation { runCatching { cancel() } }
    }
    if (!resp.isSuccessful) {
        val body = runCatching { resp.body?.string()?.take(300) }.getOrNull()
        resp.close()
        throw SakichanError.Api(resp.code, body?.ifBlank { null } ?: "HTTP ${resp.code}")
    }
    return resp
}

/** JsonElement 安全取字符串:非 primitive / null 返回 null,不抛。 */
private fun JsonElement?.strOrNull(): String? = when (this) {
    null -> null
    is JsonPrimitive -> try { content } catch (_: Exception) { null }
    else -> null
}

/** JsonObject 安全取字符串字段。 */
private fun JsonObject.str(key: String): String? = this[key].strOrNull()

/** JsonArray 安全转字符串列表。 */
private fun JsonArray.strList(): List<String> = mapNotNull { it.strOrNull() }
