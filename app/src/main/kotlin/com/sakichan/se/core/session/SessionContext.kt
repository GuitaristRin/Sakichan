package com.sakichan.se.core.session

import com.sakichan.se.core.model.Message
import java.util.*

/**
 * In-memory session context with sliding-window truncation.
 * Port of Rust sakichan-core session/context.rs.
 */
class SessionContext(
    val sessionId: String,
    val modelId: String,
    private var systemPrompt: String? = null
) {
    companion object {
        private const val MAX_CONTEXT_TOKENS = 256_000
        private const val RESPONSE_RESERVE = 4_000
        private const val CHARS_PER_TOKEN = 4
        private const val MAX_CHARS = (MAX_CONTEXT_TOKENS - RESPONSE_RESERVE) * CHARS_PER_TOKEN
    }

    private val messages: LinkedList<Message> = LinkedList()
    private var hasMemoryInjection = false
    var currentDepth: Long = 0
        private set
    var currentOrder: Long = 0
        private set
    private var branchMode: Pair<Long, Long>? = null

    fun setSystemPrompt(prompt: String?) { systemPrompt = prompt }
    fun systemPrompt(): String? = systemPrompt

    fun setDepthOrder(depth: Long, order: Long) {
        currentDepth = depth
        currentOrder = order
    }

    fun setBranchMode(depth: Long, order: Long) {
        branchMode = Pair(depth, order)
    }

    fun clearBranchMode() { branchMode = null }

    fun addUserMessage(content: String) {
        hasMemoryInjection = false
        if (branchMode != null) {
            val (d, o) = branchMode!!
            currentDepth = d
            currentOrder = o
            branchMode = null
        } else {
            currentDepth++
            currentOrder = 1
        }
        messages.addLast(Message.user(content))
    }

    fun addUserMessageWithImages(content: String, images: List<String>) {
        hasMemoryInjection = false
        if (branchMode != null) {
            val (d, o) = branchMode!!
            currentDepth = d
            currentOrder = o
            branchMode = null
        } else {
            currentDepth++
            currentOrder = 1
        }
        messages.addLast(Message.userWithImages(content, images))
    }

    fun addAssistantMessage(content: String) {
        messages.addLast(Message.assistant(content))
    }

    fun addMessage(message: Message) {
        messages.addLast(message)
    }

    fun popLastAssistant(): Message? {
        val idx = messages.indexOfLast { it.role == "assistant" }
        if (idx >= 0) {
            return messages.removeAt(idx)
        }
        return null
    }

    fun popLastUser(): Message? {
        val idx = messages.indexOfLast { it.role == "user" }
        if (idx >= 0) {
            return messages.removeAt(idx)
        }
        return null
    }

    fun injectMemoryPrompt(memoryText: String) {
        messages.removeAll { it.name == "memory_context" }
        messages.addFirst(
            Message.systemWithName("[长期记忆参考]\n$memoryText", "memory_context")
        )
        hasMemoryInjection = true
    }

    fun injectContextPrompt(contextText: String) {
        messages.removeAll { it.name == "injected_context" }
        messages.addFirst(
            Message.systemWithName(contextText, "injected_context")
        )
    }

    fun buildMessagesForModel(): List<Message> {
        if (messages.isEmpty()) return emptyList()

        val result = mutableListOf<Message>()
        systemPrompt?.let { result.add(Message.system(it)) }

        val others = messages.toMutableList()
        var totalChars = others.sumOf { it.content.length } +
            (others.sumOf { it.images.sumOf { img -> img.length } })

        while (totalChars > MAX_CHARS && others.isNotEmpty()) {
            val removed = others.removeFirst()
            totalChars -= removed.content.length
            totalChars -= removed.images.sumOf { it.length }
        }

        result.addAll(others)
        return result
    }

    fun messages(): List<Message> = messages.toList()
    fun isEmpty(): Boolean = messages.isEmpty()
    fun size(): Int = messages.size
    fun clear() { messages.clear(); hasMemoryInjection = false }
    fun loadMessages(msgs: List<Message>) {
        messages.clear()
        msgs.forEach { messages.addLast(it) }
    }
    fun hasMemoryInjection(): Boolean = hasMemoryInjection
}
