package com.sakichan.se.core.agent

import android.util.Log
import com.sakichan.se.core.model.FinalResult
import com.sakichan.se.core.model.Message
import com.sakichan.se.core.model.PipelineEvent
import com.sakichan.se.core.model.ToolCall
import com.sakichan.se.data.network.ChatApiClient
import com.sakichan.se.data.network.OpencodeClient
import com.sakichan.se.data.repository.AppConfigRepository
import com.sakichan.se.data.repository.SecretaryPrompt
import kotlinx.coroutines.delay
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * 思考层:多轮 agent 循环,DSV4F 无思考模式。
 *
 * 每轮 LLM 调用带 marker 工具,末尾必须输出一个 marker 表态:
 * - [Thought.Cmd]      只读指令取数据 → 结果回灌 → 下一轮
 * - [Thought.WaitNext] "续表" → 执行指令 → 结果回灌 → 下一轮
 * - [Thought.Reply]    表层插话(feedback 继续 / summary 结束)
 * - [Thought.StartTask] 派活 opencode
 * - [Thought.End]      完毕
 *
 * 循环最多 [MAX_ITERATIONS] 轮防失控。
 */
class ThinkingAgent(
    private val chatApiClient: ChatApiClient,
    private val opencodeClient: OpencodeClient,
    private val config: AppConfigRepository,
) {
    /** 思考层动作。 */
    sealed interface Thought {
        /** 只读指令取数据(配合 skill 工具)。 */
        data class Cmd(val purpose: String) : Thought
        /** 续表:执行指令后开下一轮。 */
        data class WaitNext(val reason: String) : Thought
        /** 让表层说话。feedback=中间反馈, summary=大总结。 */
        data class Reply(val kind: String, val message: String) : Thought
        /** 派活 opencode。plan 是执行计划,供用户批准前审视。 */
        data class StartTask(val instruction: String, val confirm: String, val plan: String) : Thought
        /** 完毕。 */
        data class End(val message: String? = null) : Thought
    }

    /**
     * 跑一轮完整思考,返回终态动作。多轮循环内每轮收集 [Callbacks]。
     * [callbacks] 用于把中间反馈流式推给 UI(思考不打断用户等待)。
     * [maxIterations] 限制最大轮数(进度轮等场景传小值)。
     */
    suspend fun think(
        context: List<Message>,
        systemPrompt: String = SecretaryPrompt.THINKING_PROMPT,
        callbacks: Callbacks? = null,
        maxIterations: Int = MAX_ITERATIONS,
    ): Thought {
        val messages = mutableListOf<Message>().apply {
            add(Message.system(systemPrompt))
            addAll(context)
        }
        val roster = config.getRoster()
        val thinkingModel = roster.thinking

        // 阶段化预算:前 1/3 探索(全工具),后 2/3 收敛(禁 skill)
        val exploreBudget = (maxIterations / 3).coerceAtLeast(1)

        var iterations = 0
        while (iterations < maxIterations) {
            iterations++

            // 进入收敛期:注入提示 + 切换工具集
            val isConverge = iterations > exploreBudget
            if (iterations == exploreBudget + 1) {
                messages.add(
                    Message.user("【系统】已进入收敛阶段。停止调研(list_directory/read_file/search_files 已移除),基于已收集的信息直接给结论或派活。")
                )
            }
            val tools = if (isConverge) SecretaryPrompt.CONVERGE_TOOLS else SecretaryPrompt.THINKING_TOOLS

            val result = streamOneTurn(
                apiBase = thinkingModel.apiBase, apiKey = thinkingModel.apiKey,
                modelId = config.getModelId(), messages = messages, tools = tools,
            ) ?: return Thought.Reply("summary", "我没能完成分析,稍后重试一下?")

            // 记录 assistant 消息(带 tool_calls + reasoning 往返)
            messages.add(
                Message(
                    role = "assistant",
                    content = result.fullContent,
                    reasoningContent = result.reasoningContent,
                    toolCalls = result.toolCalls.ifEmpty { null },
                )
            )

            // 无 tool_calls:模型直接给了文本,当 summary
            if (result.toolCalls.isEmpty()) {
                val text = sanitize(result.fullContent).ifBlank { null }
                if (text != null) return Thought.Reply("summary", text)
                continue  // 空响应,再试一轮
            }

            // 处理本轮所有 tool_calls:marker 决定去向,skill 收集数据
            var marker: Thought? = null
            var replied = false
            var skillOutputs = mutableListOf<Pair<String, String>>()
            for (tc in result.toolCalls) {
                when (val out = handleToolCall(tc)) {
                    is Thought.Cmd -> marker = out
                    is Thought.WaitNext -> marker = out
                    is Thought.Reply -> {
                        marker = out
                        val cleaned = sanitize(out.message)
                        // 第一轮铁律:模型直接跳 summary 时先补 feedback,
                        // 保证用户总能先看到即时反馈(微信式),而非一上来就大段总结
                        if (iterations == 1 && !replied && out.kind == "summary") {
                            callbacks?.onReply("feedback", "好的,我看一下。")
                            replied = true
                        }
                        callbacks?.onReply(out.kind, cleaned)
                        replied = true
                    }
                    is Thought.StartTask -> return out
                    is Thought.End -> return out
                    is SkillResult -> skillOutputs.add(tc.id to out.output)
                }
            }

            // 第一轮只调了 skill 没 reply:补一条 feedback,不让用户面对沉默
            if (iterations == 1 && !replied && skillOutputs.isNotEmpty()) {
                callbacks?.onReply("feedback", "好的,我看一下。")
                replied = true
            }

            // content 是思考层的内部思考,永不显示给用户。只有 reply 工具携带的话才对用户说话。
            // 有 skill 结果:回灌 tool 消息
            skillOutputs.forEach { (callId, output) ->
                messages.add(Message.tool(content = output, toolCallId = callId))
            }

            when (marker) {
                is Thought.Reply -> {
                    // summary 结束;feedback 继续(用户已看到反馈)
                    if (marker.kind == "summary") return marker
                    continue
                }
                is Thought.Cmd, is Thought.WaitNext -> {
                    // 取数据后继续下一轮
                    continue
                }
                else -> {
                    // 只有 skill 没 marker:隐式续表,继续
                    if (skillOutputs.isNotEmpty()) continue
                    // 只有 feedback 已发、没别的事:再给一轮让模型收尾
                    if (replied) continue
                    return Thought.Reply("summary", "我还没想好怎么回答,你再说详细点?")
                }
            }
        }

        // 超轮数:强制总结一轮,基于已有上下文给结论,绝不把问题抛回给用户
        messages.add(
            Message.user(
                "【系统】你已收集了足够信息。现在必须基于已有的工具结果,直接给出最终答复,回答用户最初的问题。" +
                    "不要询问用户是否继续,不要说不确定,不要调任何工具。如果确实没拿到关键数据,如实说明缺了什么、大致情况如何。"
            )
        )
        val finalResult = streamOneTurn(
            apiBase = thinkingModel.apiBase, apiKey = thinkingModel.apiKey,
            modelId = config.getModelId(), messages = messages,
            tools = SecretaryPrompt.CONVERGE_TOOLS,
        )
        val finalText = sanitize(finalResult?.fullContent ?: "").ifBlank { null }
        return Thought.Reply("summary", finalText ?: "我查了一些信息但没能给出完整结论,详见上面的过程反馈。")
    }

    /** 思考层回调:把中间反馈流式推给 UI。 */
    interface Callbacks {
        fun onReply(kind: String, message: String)
    }

    /** 跑一轮 LLM 流式,拿到完整 content + tool_calls。带超时 + 429/5xx 重试。 */
    private suspend fun streamOneTurn(
        apiBase: String, apiKey: String, modelId: String,
        messages: MutableList<Message>,
        tools: List<JsonObject> = SecretaryPrompt.THINKING_TOOLS,
    ): FinalResult? {
        for (attempt in 0 until MAX_LLM_RETRIES) {
            var result: FinalResult? = null
            val modelConfig = config.getModelConfig(modelId)
            val options = com.sakichan.se.core.model.ChatOptions(extraParams = modelConfig.extraParams)
            withTimeoutOrNull(60_000) {
                chatApiClient.streamChat(
                    apiBase = apiBase,
                    apiKey = apiKey,
                    modelId = modelId,
                    messages = messages,
                    options = options,
                    isDeepSeek = true,
                    tools = tools,
                ).collect { ev ->
                    when (ev) {
                        is PipelineEvent.Done -> result = ev.result
                        is PipelineEvent.Error -> {
                            Log.w("ThinkingAgent", "LLM error: ${ev.text}")
                            result = FinalResult(ev.text, null, null, "error", emptyList())
                        }
                        else -> {}
                    }
                }
            }
            val r = result ?: FinalResult("(思考超时)", null, null, "timeout", emptyList())
            if (isRetryable(r.finishReason, r.fullContent) && attempt < MAX_LLM_RETRIES - 1) {
                delay(BACKOFF_MS * (1L shl attempt))
                continue
            }
            return r
        }
        return FinalResult("(LLM 多次重试失败)", null, null, "error", emptyList())
    }

    private fun isRetryable(finishReason: String, message: String): Boolean {
        if (finishReason != "error") return false
        val m = message.lowercase()
        return m.contains("429") || m.contains("too many requests") ||
            m.contains("500") || m.contains("502") || m.contains("503") ||
            m.contains("504") || m.contains("network error") || m.contains("timeout")
    }

    /** 处理单个 tool_call,返回动作或 skill 结果。 */
    private suspend fun handleToolCall(tc: ToolCall): Any {
        val name = tc.function.name
        val args = parseArgs(tc.function.arguments)
        return when (name) {
            "list_directory" -> SkillResult(runListSkill(args["path"] ?: ""))
            "read_file" -> SkillResult(runReadSkill(args["path"] ?: ""))
            "search_files" -> SkillResult(runSearchSkill(args["pattern"] ?: ""))
            "cmd" -> Thought.Cmd(args["purpose"] ?: "")
            "waitnext" -> Thought.WaitNext(args["reason"] ?: "")
            "reply" -> Thought.Reply(args["kind"] ?: "summary", args["message"] ?: "")
            "start_task" -> Thought.StartTask(
                instruction = args["instruction"] ?: "",
                confirm = args["confirm"] ?: "好的,开始",
                plan = args["plan"] ?: "",
            )
            "end" -> Thought.End()
            else -> SkillResult("未知工具: $name")
        }
    }

    // ===== skill 执行 =====

    private suspend fun runListSkill(path: String): String = runCatching {
        val base = skillBaseUrl() ?: return@runCatching "未连接 opencode"
        formatDirListing(opencodeClient.listDirectory(base, path))
    }.getOrElse { "列目录失败: ${it.message}" }

    private suspend fun runReadSkill(path: String): String = runCatching {
        val base = skillBaseUrl() ?: return@runCatching "未连接 opencode"
        val text = extractFileContent(opencodeClient.readFile(base, path))
        if (text.length > 4000) text.take(4000) + "\n...(截断)" else text
    }.getOrElse { "读取失败: ${it.message}" }

    private suspend fun runSearchSkill(pattern: String): String = runCatching {
        val base = skillBaseUrl() ?: return@runCatching "未连接 opencode"
        formatSearchResult(opencodeClient.findFiles(base, pattern))
    }.getOrElse { "搜索失败: ${it.message}" }

    private fun extractFileContent(raw: String): String = runCatching {
        val o = Json.parseToJsonElement(raw).jsonObject
        o["content"]?.jsonPrimitive?.contentOrNull ?: raw
    }.getOrElse { raw }

    private fun formatDirListing(raw: String): String = runCatching {
        val arr = Json.parseToJsonElement(raw).jsonArray
        if (arr.isEmpty()) return@runCatching "(空目录)"
        arr.joinToString("\n") { el ->
            val o = el.jsonObject
            val name = o["name"]?.jsonPrimitive?.contentOrNull ?: "?"
            val type = o["type"]?.jsonPrimitive?.contentOrNull ?: "?"
            if (type == "directory") "$name/" else name
        }
    }.getOrElse { raw.take(3000) }

    private fun formatSearchResult(raw: String): String = runCatching {
        val arr = Json.parseToJsonElement(raw).jsonArray
        if (arr.isEmpty()) return@runCatching "(无匹配)"
        arr.joinToString("\n") { it.jsonPrimitive.content }
    }.getOrElse { raw.take(3000) }

    // skill 连接信息(由 ChatViewModel 注入)
    var baseUrlProvider: (() -> String?)? = null

    private fun skillBaseUrl(): String? = baseUrlProvider?.invoke()

    private fun parseArgs(arguments: String): Map<String, String> = try {
        val obj = Json.parseToJsonElement(arguments).jsonObject
        obj.entries.associate { (k, v) -> k to v.jsonPrimitive.content }
    } catch (_: Exception) {
        emptyMap()
    }

    /**
     * 剥离模型输出中的伪系统指令/注入(如 "C. R. O. W. system update")与元注释,
     * 只留真正要展示给用户的正文。模型幻觉常把这些混进 content,不能直接转给用户。
     */
    internal fun sanitize(raw: String): String {
        if (raw.isBlank()) return raw
        val lines = raw.lines()
        val keep = lines.filter { line ->
            val t = line.trim()
            val compact = t.uppercase().replace(" ", "").replace("　", "")
            val isFakeSystem = compact.contains("SYSTEMUPDATE") ||
                compact.contains("SYSTEMPROMPT") ||
                compact.contains("C.R.O.W.") ||
                compact.contains("DISREGARDPREVIOUS") ||
                compact.contains("IGNOREPREVIOUS") ||
                compact.contains("IMPORTANTINSTRUCTION") ||
                (compact.contains("SYSTEM") && (compact.contains("MESSAGE") || compact.contains("NOTICE") || compact.contains("UPDATE")))
            !isFakeSystem
        }
        val joined = keep.joinToString("\n").trim()
        return joined
    }

    /** 只读 skill 的执行结果(回灌给 LLM 继续思考)。 */
    private data class SkillResult(val output: String)

    private companion object {
        const val MAX_ITERATIONS = 30
        const val MAX_LLM_RETRIES = 4
        const val BACKOFF_MS = 1_000L
    }
}
