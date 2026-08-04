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
    suspend fun createSession(baseUrl: String, req: OcSessionCreateRequest = OcSessionCreateRequest()): OcSession {
        val body = json.encodeToString(OcSessionCreateRequest.serializer(), req)
            .toRequestBody(JSON)
        val resp = okHttpClient.newCall(
            Request.Builder().url("${trimSlash(baseUrl)}/session").post(body).build()
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
     * 订阅 session 事件流(SSE)。这是核心:实时拿 token 输出 / 工具调用 / 权限请求。
     * 端点 `/api/session/:id/event`,首事件后持续推送,直到 [session.idle] 或出错。
     *
     * 防御性解析:服务端事件可能为 `{id,type,properties}` 或扁平 `{type,...}`,
     * 统一归一为 [OcEvent]。取消订阅即取消 EventSource。
     */
    fun events(baseUrl: String, sessionId: String): Flow<OcEvent> = callbackFlow {
        val url = "${trimSlash(baseUrl)}/api/session/$sessionId/event"
        val request = Request.Builder().url(url).get().build()
        val factory = EventSources.createFactory(okHttpClient)

        val listener = object : EventSourceListener() {
            override fun onEvent(source: EventSource, id: String?, type: String?, data: String) {
                if (data.isBlank()) return
                try {
                    val parsed = parseEvent(data, type)
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
                        response != null -> "SSE HTTP ${response.code}"
                        else -> "SSE unknown error"
                    }
                    trySend(OcEvent.SessionError(sessionId, msg))
                }
            }

            override fun onClosed(source: EventSource) {
                channel.close()
            }
        }

        val source = factory.newEventSource(request, listener)
        awaitClose { source.cancel() }
    }

    /**
     * 归一化解析单条 SSE data。
     * - 若 JSON 含 `type` 字段:用其值判定事件类型,payload 在 `properties` 或顶层。
     * - 否则用 SSE 的 `event:` 头(type 形参)。
     */
    private fun parseEvent(data: String, sseType: String?): OcEvent {
        val obj = json.parseToJsonElement(data).jsonObject
        val type = obj.str("type") ?: sseType ?: return OcEvent.Other(null, "unknown", obj)

        val props = obj["properties"]?.jsonObject ?: obj
        val sid = props.str("sessionID") ?: obj.str("sessionID")

        return when (type) {
            OcEvent.TYPE_TEXT_DELTA -> OcEvent.TextDelta(
                sessionID = sid,
                messageID = props.str("assistantMessageID"),
                textID = props.str("textID"),
                delta = props.str("delta") ?: "",
            )

            OcEvent.TYPE_REASONING_DELTA -> OcEvent.ReasoningDelta(
                sessionID = sid,
                messageID = props.str("assistantMessageID"),
                reasoningID = props.str("reasoningID"),
                delta = props.str("delta") ?: "",
            )

            OcEvent.TYPE_TOOL_CALLED -> OcEvent.ToolCalled(
                sessionID = sid,
                messageID = props.str("assistantMessageID"),
                callID = props.str("callID"),
                tool = props.str("tool") ?: "tool",
                input = props["input"],
            )

            OcEvent.TYPE_TOOL_SUCCESS -> OcEvent.ToolSuccess(
                sessionID = sid,
                messageID = props.str("assistantMessageID"),
                callID = props.str("callID"),
                output = props.str("output"),
            )

            OcEvent.TYPE_TOOL_FAILED, OcEvent.TYPE_STEP_FAILED -> OcEvent.ToolFailed(
                sessionID = sid,
                messageID = props.str("assistantMessageID"),
                callID = props.str("callID"),
                error = props["error"]?.let { err ->
                    when (err) {
                        is JsonObject -> err.str("message") ?: err.toString()
                        else -> err.strOrNull() ?: err.toString()
                    }
                } ?: type,
            )

            OcEvent.TYPE_TOOL_PROGRESS -> OcEvent.Other(sid, type, obj)

            OcEvent.TYPE_STEP_STARTED -> OcEvent.StepStarted(
                sessionID = sid,
                messageID = props.str("assistantMessageID"),
                agent = props.str("agent"),
            )

            OcEvent.TYPE_STEP_ENDED -> OcEvent.StepEnded(
                sessionID = sid,
                messageID = props.str("assistantMessageID"),
                reason = props.str("reason"),
            )

            OcEvent.TYPE_PERMISSION_ASKED -> OcEvent.PermissionAsked(
                sessionID = sid,
                requestID = props.str("id") ?: "",
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

            else -> OcEvent.Other(sid, type, obj)
        }
    }

    private fun trimSlash(url: String): String = url.trimEnd('/')

    companion object {
        private val JSON = "application/json".toMediaType()
        private val EMPTY = "".toRequestBody(JSON)
    }
}

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
