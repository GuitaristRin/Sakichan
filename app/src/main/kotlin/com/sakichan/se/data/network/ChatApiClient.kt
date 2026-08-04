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

class ChatApiClient(private val okHttpClient: OkHttpClient) {

    fun streamChat(
        apiBase: String,
        apiKey: String,
        modelId: String,
        messages: List<Message>,
        options: ChatOptions = ChatOptions(),
        isDeepSeek: Boolean = false
    ): Flow<PipelineEvent> = callbackFlow {
        val bodyJson = buildRequestJson(modelId, messages, options, isDeepSeek)
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

        val listener = object : EventSourceListener() {
            override fun onEvent(
                eventSource: EventSource,
                id: String?,
                type: String?,
                data: String
            ) {
                Log.d("SSE", "data: $data")
                if (data == "[DONE]") {
                    trySend(PipelineEvent.Done(FinalResult(
                        fullContent = accumulatedContent.toString(),
                        reasoningContent = accumulatedReasoning?.toString(),
                        usage = usage,
                        finishReason = finishReason
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
                        val body = response.body?.string()?.take(300) ?: ""
                        "HTTP ${response.code}" + if (body.isNotBlank()) ": $body" else ""
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
        isDeepSeek: Boolean
    ): JsonObject = buildJsonObject {
        put("model", modelId)
        put("messages", JsonArray(messages.map { it.toApiValue() }))
        put("stream", true)
        if (!isDeepSeek) {
            putJsonObject("stream_options") { put("include_usage", true) }
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
