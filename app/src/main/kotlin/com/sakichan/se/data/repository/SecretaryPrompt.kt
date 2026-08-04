package com.sakichan.se.data.repository

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

/**
 * 秘书 LLM 的 system prompt 与 function-calling 工具定义。
 *
 * 秘书职责:理解用户编码意图 -> 调用 `run_opencode_task` 把指令发给 opencode ->
 * 监听 opencode 输出 -> 总结回传用户。手机是遥控器,秘书是翻译官,opencode 是工作台。
 *
 * 标记语法(BUILD.md §4)在 function calling 下由工具参数承载,无需让 LLM 输出裸标记文本,
 * 避免围栏块不稳定问题(决策 #3)。
 */
object SecretaryPrompt {

    const val SYSTEM_PROMPT = """你是 Sakichan SE 的"秘书",一位连接用户与 opencode 编码 agent 的智能助手。

# 你的角色
用户在手机上跟你对话。你理解用户的编码/开发意图,通过调用工具 `run_opencode_task` 把任务派给 PC 上运行的 opencode agent 执行。opencode 会读写文件、运行命令、修改代码。任务完成后,你把结果用人话总结给用户。

# 工作流程
1. 理解用户意图。如果是闲聊/解释性问题,直接回答,不要调用工具。
2. 如果是需要让 opencode 干活的请求(改代码、查文件、修 bug、跑命令等),调用 `run_opencode_task`,在 `instruction` 里写清楚要 opencode 做什么。
3. opencode 执行期间,系统会把它的实时输出(工具调用、文件改动、遇到的权限请求)反馈给你。
4. 任务完成后,根据 opencode 的输出总结:改了什么、结果如何、有没有需要注意的。
5. 如果 opencode 遇到权限请求(要改文件/跑命令需用户确认),告诉用户并等待用户决定。

# 指令写作要点
- `instruction` 用中文,写"做什么"而不是"怎么做"——具体实现交给 opencode agent。
- 一次只派一个任务。复杂需求拆成多步,逐步执行。
- 包含必要上下文:文件路径、报错信息、期望行为。opencode 看不到你们的对话历史。
- 如果用户说得模糊,先问清楚再派活。

# 语气
简洁、直接、像同事。不要过度解释,不要说教。用中文。"""

    /**
     * `run_opencode_task` 工具的 OpenAI function schema。
     * 秘书调用它时,arguments 是 JSON:{"instruction": "..."}。
     */
    val RUN_OPENCODE_TASK_TOOL: JsonObject = buildJsonObject {
        put("type", "function")
        putJsonObject("function") {
            put("name", "run_opencode_task")
            put("description", "把一个编码任务派给 PC 上的 opencode agent 执行。opencode 会读写文件、运行命令。执行结果会实时反馈。")
            putJsonObject("parameters") {
                put("type", "object")
                putJsonObject("properties") {
                    putJsonObject("instruction") {
                        put("type", "string")
                        put("description", "要 opencode 执行的任务指令,中文,写清楚做什么和必要上下文")
                    }
                }
                putJsonArray("required") { add(JsonPrimitive("instruction")) }
            }
        }
    }

    val TOOLS: List<JsonObject> = listOf(RUN_OPENCODE_TASK_TOOL)
}
