package com.sakichan.se.data.network

import android.util.Log
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.serialization.json.*
import com.sakichan.se.core.model.*
import okhttp3.*
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.sse.EventSource
import okhttp3.sse.EventSourceListener
import okhttp3.sse.EventSources

/**
 * 秘书 LLM 流式聊天客户端。经 SenseNova token 端点调用 DeepSeek V4 Flash。
 *
 * 支持 function calling:传 [tools] 后,流式 delta 里的 `tool_calls` 增量会被
 * 归并,最终随 [FinalResult.toolCalls] 一次返回(同时逐段发 [PipelineEvent.ToolCallDelta]。
 * 秘书用它产出 `run_opencode_task` 指令,ChatViewModel 据此调度 opencode server。
 */
class ChatApiClient(private val okHttpClient: OkHttpClient) {

    fun streamChat(
        apiBase: String,
        apiKey: String,
        modelId: String,
        messages: List<Message>,
        options: ChatOptions = ChatOptions(),
        isDeepSeek: Boolean = false,
        tools: List<JsonObject> = emptyList(),
    ): Flow<PipelineEvent> = callbackFlow {
        val bodyJson = buildRequestJson(modelId, messages, options, isDeepSeek, tools)
        val bodyString = bodyJson.toString()
        val request = Request.Builder()
            .url(apiBase)
            .header("Authorization", "Bearer $apiKey")
            .header("Content-Type", "application/json")
            .post(bodyString.toRequestBody("application/json".toMediaType()))
            .build()

        val factory = EventSources.createFactory(okHttpClient)
        val accumulatedContent = StringBuilder()
        var accumulatedReasoning: StringBuilder? = null
        var finishReason = "stop"
        var usage: UsageInfo? = null
        // index -> 累积中的 tool call
        val toolCallAccum = LinkedHashMap<Int, ToolCallAccum>()

        val listener = object : EventSourceListener() {
            override fun onEvent(
                eventSource: EventSource,
                id: String?,
                type: String?,
                data: String
            ) {
                Log.d("SSE", "data: $data")
                if (data == "[DONE]") {
                    val toolCalls = toolCallAccum.values.mapNotNull { it.build() }
                    trySend(PipelineEvent.Done(FinalResult(
                        fullContent = accumulatedContent.toString(),
                        reasoningContent = accumulatedReasoning?.toString(),
                        usage = usage,
                        finishReason = finishReason,
                        toolCalls = toolCalls,
                    )))
                    return
                }
                try {
                    val json = Json.parseToJsonElement(data).jsonObject

                    json["choices"]?.jsonArray?.forEach { choiceObj ->
                        val choice = choiceObj.jsonObject
                        val delta = choice["delta"]?.jsonObject ?: return@forEach

                        delta["content"]?.jsonPrimitive?.contentOrNull?.let { token ->
                            if (token.isNotEmpty()) {
                                accumulatedContent.append(token)
                                trySend(PipelineEvent.Token(token))
                            }
                        }

                        delta["reasoning_content"]?.jsonPrimitive?.contentOrNull?.let { token ->
                            if (token.isNotEmpty()) {
                                if (accumulatedReasoning == null) {
                                    accumulatedReasoning = StringBuilder()
                                }
                                accumulatedReasoning!!.append(token)
                                trySend(PipelineEvent.ReasoningToken(token))
                            }
                        }

                        // function calling 增量:delta.tool_calls[] 带 index,逐段拼接 arguments。
                        // SenseNova 后续 chunk 的 id/name 是空字符串,不能覆盖首块的正确值。
                        delta["tool_calls"]?.jsonArray?.forEach { tcElem ->
                            val tc = tcElem.jsonObject
                            val idx = tc["index"]?.jsonPrimitive?.intOrNull ?: 0
                            val accum = toolCallAccum.getOrPut(idx) { ToolCallAccum() }
                            tc["id"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotBlank() }?.let { accum.id = it }
                            tc["function"]?.jsonObject?.let { fn ->
                                fn["name"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotBlank() }?.let { accum.name = it }
                                fn["arguments"]?.jsonPrimitive?.contentOrNull?.let { arg ->
                                    accum.arguments.append(arg)
                                    trySend(PipelineEvent.ToolCallDelta(idx, accum.id, accum.name, arg))
                                }
                            }
                        }

                        choice["finish_reason"]?.jsonPrimitive?.contentOrNull?.let {
                            if (it != "null" && it.isNotEmpty()) {
                                finishReason = it
                            }
                        }
                    }

                    json["usage"]?.jsonObject?.let { u ->
                        usage = UsageInfo(
                            promptTokens = u["prompt_tokens"]?.jsonPrimitive?.intOrNull ?: 0,
                            completionTokens = u["completion_tokens"]?.jsonPrimitive?.intOrNull ?: 0,
                            totalTokens = u["total_tokens"]?.jsonPrimitive?.intOrNull ?: 0
                        )
                    }
                } catch (e: Exception) {
                    Log.w("SSE", "Parse error: ${e.message}", e)
                    trySend(PipelineEvent.Error("Parse error: ${e.message}"))
                }
            }

            override fun onFailure(eventSource: EventSource, t: Throwable?, response: Response?) {
                val msg = when {
                    t != null -> "Network error: ${t.message}"
                    response != null -> {
                        val body = response.body?.string().orEmpty()
                        Log.w("SSE", "HTTP ${response.code}: $body")
                        "HTTP ${response.code}: ${body.take(500)}"
                    }
                    else -> "Unknown SSE error"
                }
                trySend(PipelineEvent.Error(msg))
            }

            override fun onClosed(eventSource: EventSource) {
                channel.close()
            }
        }

        val eventSource = factory.newEventSource(request, listener)

        awaitClose {
            eventSource.cancel()
        }
    }

    private fun buildRequestJson(
        modelId: String,
        messages: List<Message>,
        options: ChatOptions,
        isDeepSeek: Boolean,
        tools: List<JsonObject>,
    ): JsonObject = buildJsonObject {
        put("model", modelId)
        put("messages", JsonArray(messages.map { it.toApiValue() }))
        put("stream", true)
        putJsonObject("stream_options") { put("include_usage", true) }
        if (tools.isNotEmpty()) {
            put("tools", JsonArray(tools))
            put("tool_choice", "auto")
        }
        options.maxTokens?.let { put("max_tokens", it) }
        options.temperature?.let { put("temperature", it.toDouble()) }
        options.topP?.let { put("top_p", it.toDouble()) }
        if (isDeepSeek) {
            options.extraParams.forEach { entry: Map.Entry<String, JsonElement> ->
                put(entry.key, entry.value)
            }
        }
    }
}

private class ToolCallAccum {
    var id: String? = null
    var name: String? = null
    val arguments = StringBuilder()

    fun build(): ToolCall? {
        val id = id?.takeIf { it.isNotBlank() } ?: return null
        val name = name?.takeIf { it.isNotBlank() } ?: return null
        val args = arguments.toString()
        // SenseNova 校验 tool_call 的 name/arguments 非空;空参数视为未完整返回,丢弃
        if (args.isBlank()) return null
        return ToolCall(id = id, type = "function", function = ToolFunction(name, args))
    }
}
