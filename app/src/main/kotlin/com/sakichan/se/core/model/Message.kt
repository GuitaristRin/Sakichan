package com.sakichan.se.core.model

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.*

@Serializable
data class Message(
    val role: String,
    val content: String,
    val reasoningContent: String? = null,
    val toolCalls: List<ToolCall>? = null,
    val toolCallId: String? = null,
    val name: String? = null,
    @kotlinx.serialization.Transient val images: List<String> = emptyList()
) {
    companion object {
        fun user(content: String) = Message(role = "user", content = content)
        fun userWithImages(content: String, images: List<String>) =
            Message(role = "user", content = content, images = images)
        fun assistant(content: String) = Message(role = "assistant", content = content)
        fun system(content: String) = Message(role = "system", content = content)
        fun systemWithName(content: String, name: String) =
            Message(role = "system", content = content, name = name)
        fun tool(content: String, toolCallId: String) =
            Message(role = "tool", content = content, toolCallId = toolCallId)
    }

    fun toApiValue(): JsonObject = buildJsonObject {
        put("role", role)
        if (images.isEmpty()) {
            put("content", content)
        } else {
            put("content", buildJsonArray {
                if (content.isNotBlank()) {
                    addJsonObject { put("type", "text"); put("text", content) }
                }
                images.forEach { uri ->
                    addJsonObject {
                        put("type", "image_url")
                        putJsonObject("image_url") { put("url", uri) }
                    }
                }
            })
        }
        reasoningContent?.let { put("reasoning_content", it) }
        toolCalls?.let { put("tool_calls", buildJsonArray { it.forEach { tc -> add(tc.toJson()) } }) }
        toolCallId?.let { put("tool_call_id", it) }
        name?.let { put("name", it) }
    }
}

@Serializable
data class ToolCall(
    val id: String,
    @Serializable val type: String = "function",
    val function: ToolFunction
) {
    fun toJson(): JsonObject = buildJsonObject {
        put("id", id)
        put("type", type)
        putJsonObject("function") {
            put("name", function.name)
            put("arguments", function.arguments)
        }
    }
}

@Serializable
data class ToolFunction(
    val name: String,
    val arguments: String
)
