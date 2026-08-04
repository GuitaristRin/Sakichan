package com.sakichan.se.core.model

import kotlinx.serialization.json.JsonElement

data class ChatOptions(
    val maxTokens: Int? = null,
    val temperature: Float? = null,
    val topP: Float? = null,
    val extraParams: Map<String, JsonElement> = emptyMap()
)

data class FinalResult(
    val fullContent: String,
    val reasoningContent: String?,
    val usage: UsageInfo?,
    val finishReason: String
)

data class UsageInfo(
    val promptTokens: Int,
    val completionTokens: Int,
    val totalTokens: Int
)

data class Session(
    val id: String,
    val title: String?,
    val createdAt: Long,
    val updatedAt: Long,
    val modelId: String,
    val systemPrompt: String?,
    val activeDepth: Long,
    val activeOrder: Long
)

data class ConversationSummary(
    val id: Long,
    val sessionId: String,
    val depth: Long,
    val orderAt: Long,
    val content: String,
    val messagesCount: Int,
    val summarizedAt: Long
)
