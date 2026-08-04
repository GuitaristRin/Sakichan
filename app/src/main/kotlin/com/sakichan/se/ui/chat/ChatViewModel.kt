package com.sakichan.se.ui.chat

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.sakichan.se.connection.ConnectionManager
import com.sakichan.se.core.model.*
import com.sakichan.se.core.session.SessionContext
import com.sakichan.se.data.network.ChatApiClient
import com.sakichan.se.data.network.OpencodeClient
import com.sakichan.se.data.repository.AppConfigRepository
import com.sakichan.se.data.repository.SecretaryPrompt
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.util.UUID

/**
 * 编排层:用户 -> 秘书 LLM -> opencode server -> 秘书总结 -> 用户。
 *
 * 秘书用 function calling 调 `run_opencode_task`。收到 tool_call 后:
 * 1. 在 opencode server 创建/复用 session
 * 2. prompt_async 发指令
 * 3. 订阅 session 事件流,实时把 token/工具/权限反馈进 UI
 * 4. session.idle 后,把 opencode 输出拼成 tool result 回灌秘书做总结
 *
 * opencode 权限请求会挂起任务,经 [replyPermission] 由用户在 UI approve/deny。
 */
class ChatViewModel(
    private val chatApiClient: ChatApiClient,
    private val opencodeClient: OpencodeClient,
    private val config: AppConfigRepository,
    private val connection: ConnectionManager,
) : ViewModel() {

    private val _uiState = MutableStateFlow(ChatUiState())
    val uiState: StateFlow<ChatUiState> = _uiState.asStateFlow()

    private val sessionContext = SessionContext(
        sessionId = "local",
        modelId = "deepseek-v4-flash",
        systemPrompt = SecretaryPrompt.SYSTEM_PROMPT,
    )
    private var opencodeSessionId: String? = null
    private var secretaryJob: Job? = null

    /** 当前活跃机器的 baseUrl;未连接时返回 null,发送消息前做保护。 */
    private fun activeBaseUrl(): String? = connection.activeBaseUrl

    fun onInputChange(text: String) {
        _uiState.update { it.copy(inputText = text) }
    }

    fun send() {
        val text = _uiState.value.inputText.trim()
        if (text.isEmpty() || _uiState.value.isLoading) return
        if (activeBaseUrl() == null) {
            addError("未连接 opencode,请先到连接页选择机器")
            return
        }
        _uiState.update { it.copy(inputText = "", isLoading = true) }

        val userItem = ChatItem.User(id = genId(), text = text)
        _uiState.update { it.copy(items = it.items + userItem) }
        sessionContext.addUserMessage(text)

        runSecretaryTurn()
    }

    /** 用户对权限请求的回复:approve=true -> once/always;false -> reject。 */
    fun replyPermission(approve: Boolean, always: Boolean) {
        val pending = _uiState.value.pendingPermission ?: return
        val response = when {
            !approve -> "reject"
            always -> "always"
            else -> "once"
        }
        _uiState.update { it.copy(pendingPermission = null) }
        val sid = opencodeSessionId ?: return
        val url = activeBaseUrl() ?: return
        viewModelScope.launch {
            runCatching { opencodeClient.replyPermission(url, sid, pending.request.id, response) }
                .onFailure { e -> addError("权限回复失败: ${e.message}") }
        }
    }

    private fun runSecretaryTurn() {
        secretaryJob?.cancel()
        secretaryJob = viewModelScope.launch {
            val modelId = config.getModelId()
            val modelConfig = config.getModelConfig(modelId)
            if (modelConfig.apiKey.isBlank()) {
                _uiState.update { it.copy(isLoading = false, notConfigured = true) }
                return@launch
            }

            val secretaryItem = ChatItem.Secretary(
                id = genId(), text = "", reasoning = null, streaming = true,
            )
            _uiState.update { it.copy(items = it.items + secretaryItem) }

            val options = ChatOptions(extraParams = modelConfig.extraParams)
            var fullContent = StringBuilder()
            var reasoning = StringBuilder()
            var result: FinalResult? = null

            chatApiClient.streamChat(
                apiBase = modelConfig.apiBase,
                apiKey = modelConfig.apiKey,
                modelId = modelId,
                messages = sessionContext.buildMessagesForModel(),
                options = options,
                isDeepSeek = true,
                tools = SecretaryPrompt.TOOLS,
            ).collect { ev ->
                when (ev) {
                    is PipelineEvent.Token -> {
                        fullContent.append(ev.text)
                        updateSecretary(secretaryItem.id, fullContent.toString(), reasoning.toString(), true)
                    }
                    is PipelineEvent.ReasoningToken -> {
                        reasoning.append(ev.text)
                        updateSecretary(secretaryItem.id, fullContent.toString(), reasoning.toString(), true)
                    }
                    is PipelineEvent.Done -> result = ev.result
                    is PipelineEvent.Error -> {
                        updateSecretary(secretaryItem.id, fullContent.toString(), reasoning.toString(), false)
                        addError(ev.text)
                        _uiState.update { it.copy(isLoading = false) }
                        return@collect
                    }
                    is PipelineEvent.ToolCallDelta -> { /* 累积中,不单独展示 */ }
                    else -> {}
                }
            }

            val finalResult = result ?: run {
                _uiState.update { it.copy(isLoading = false) }
                return@launch
            }

            // 记录秘书回复进上下文(含 tool_calls,让下一轮 LLM 知道它派了活)
            sessionContext.addAssistantMessage(finalResult.fullContent)
            updateSecretary(secretaryItem.id, finalResult.fullContent, reasoning.toString().ifBlank { null }, false)

            if (finalResult.toolCalls.isNotEmpty()) {
                // 有工具调用:执行 opencode 任务
                finalResult.toolCalls.forEach { tc ->
                    if (tc.function.name == "run_opencode_task") {
                        val instruction = parseInstruction(tc.function.arguments)
                        if (instruction != null) {
                            executeOpencodeTask(secretaryItem.id, instruction, tc.id)
                        }
                    }
                }
            } else {
                _uiState.update { it.copy(isLoading = false) }
            }
        }
    }

    /** 执行一个 opencode 任务:创建 session -> 发指令 -> 订阅事件流 -> 总结。 */
    private suspend fun executeOpencodeTask(secretaryId: String, instruction: String, toolCallId: String) {
        val url = activeBaseUrl() ?: return
        val taskId = genId()
        val taskItem = ChatItem.Task(taskId, instruction, TaskStatus.RUNNING)
        _uiState.update { it.copy(items = it.items + taskItem) }

        val taskOutput = StringBuilder()

        try {
            val sid = ensureOpencodeSession(url)
            _uiState.update { it.copy(sessionId = sid) }

            opencodeClient.promptAsync(url, sid, OcMessageRequest(parts = listOf(OcTextPartInput(text = instruction))))

            opencodeClient.events(url, sid).collect { ev ->
                when (ev) {
                    is OcEvent.TextDelta -> {
                        taskOutput.append(ev.delta)
                        updateTask(taskId, TaskStatus.RUNNING, taskOutput.toString().takeLast(200))
                    }
                    is OcEvent.ReasoningDelta -> { /* opencode 思考块,暂不展示 */ }
                    is OcEvent.ToolCalled -> updateTask(taskId, TaskStatus.TOOL, ev.tool)
                    is OcEvent.ToolSuccess -> updateTask(taskId, TaskStatus.RUNNING, taskOutput.toString().takeLast(200))
                    is OcEvent.ToolFailed -> updateTask(taskId, TaskStatus.RUNNING, "工具失败: ${ev.error}")
                    is OcEvent.PermissionAsked -> {
                        _uiState.update { it.copy(pendingPermission = PendingPermission(
                            OcPermissionRequest(
                                id = ev.requestID,
                                sessionID = sid,
                                permission = ev.permission,
                                patterns = ev.patterns,
                                tool = ev.tool?.let { OcPermissionToolRef(it.messageID, it.callID) },
                            ), taskId,
                        )) }
                    }
                    is OcEvent.SessionError -> {
                        updateTask(taskId, TaskStatus.FAILED, ev.error)
                        return@collect
                    }
                    is OcEvent.SessionIdle -> {
                        // 任务完成
                    }
                    else -> {}
                }
            }

            updateTask(taskId, TaskStatus.DONE, null)

            // 把 opencode 输出作为 tool result 回灌秘书,让它总结
            val toolResultMsg = Message.tool(
                content = if (taskOutput.isBlank()) "任务执行完毕(无文本输出)" else taskOutput.toString(),
                toolCallId = toolCallId,
            )
            sessionContext.addMessage(toolResultMsg)

            // 再跑一轮秘书,让它总结 opencode 结果
            _uiState.update { it.copy(isLoading = true) }
            runSecretarySummaryTurn()
        } catch (e: Exception) {
            updateTask(taskId, TaskStatus.FAILED, e.message)
            _uiState.update { it.copy(isLoading = false) }
        }
    }

    /** 总结轮:秘书基于 tool result 产出面向用户的总结,不再带 tools(避免无限派活)。 */
    private suspend fun runSecretarySummaryTurn() {
        val modelId = config.getModelId()
        val modelConfig = config.getModelConfig(modelId)
        val summaryItem = ChatItem.Secretary(id = genId(), text = "", reasoning = null, streaming = true)
        _uiState.update { it.copy(items = it.items + summaryItem) }

        val options = ChatOptions(extraParams = modelConfig.extraParams)
        var fullContent = StringBuilder()
        var reasoning = StringBuilder()

        chatApiClient.streamChat(
            apiBase = modelConfig.apiBase,
            apiKey = modelConfig.apiKey,
            modelId = modelId,
            messages = sessionContext.buildMessagesForModel(),
            options = options,
            isDeepSeek = true,
            tools = emptyList(),  // 总结轮不给工具,强制输出文本
        ).collect { ev ->
            when (ev) {
                is PipelineEvent.Token -> {
                    fullContent.append(ev.text)
                    updateSecretary(summaryItem.id, fullContent.toString(), reasoning.toString(), true)
                }
                is PipelineEvent.ReasoningToken -> {
                    reasoning.append(ev.text)
                    updateSecretary(summaryItem.id, fullContent.toString(), reasoning.toString(), true)
                }
                is PipelineEvent.Done -> {
                    sessionContext.addAssistantMessage(ev.result.fullContent)
                    updateSecretary(summaryItem.id, ev.result.fullContent, reasoning.toString().ifBlank { null }, false)
                }
                is PipelineEvent.Error -> {
                    updateSecretary(summaryItem.id, fullContent.toString(), reasoning.toString(), false)
                    addError(ev.text)
                }
                else -> {}
            }
        }
        _uiState.update { it.copy(isLoading = false) }
    }

    private suspend fun ensureOpencodeSession(url: String): String {
        opencodeSessionId?.let { return it }
        val session = opencodeClient.createSession(url)
        opencodeSessionId = session.id
        return session.id
    }

    // ===== 抽屉 / session 树管理 =====

    fun openDrawer() {
        _uiState.update { it.copy(drawerOpen = true) }
        refreshSessionTree()
    }

    fun closeDrawer() {
        _uiState.update { it.copy(drawerOpen = false) }
    }

    /** 拉取当前机器的「机器-项目-session」树,供抽屉展示。 */
    fun refreshSessionTree() {
        val machine = connection.activeMachine ?: return
        _uiState.update { it.copy(treeLoading = true) }
        viewModelScope.launch {
            runCatching { opencodeClient.buildSessionTree(machine) }
                .onSuccess { tree -> _uiState.update { it.copy(sessionTree = tree, treeLoading = false) } }
                .onFailure { e -> _uiState.update { it.copy(treeLoading = false) } }
        }
    }

    /** 切换到一个已存在的 session:重建上下文,清空当前聊天区。 */
    fun openSession(sessionId: String) {
        val machine = connection.activeMachine ?: return
        closeDrawer()
        viewModelScope.launch {
            runCatching {
                opencodeClient.openSessionContext(machine, sessionId, SecretaryPrompt.SYSTEM_PROMPT)
            }.onSuccess { ctx ->
                sessionContext.loadMessages(emptyList())
                opencodeSessionId = sessionId
                _uiState.value = ChatUiState(sessionId = sessionId)
            }.onFailure { e ->
                addError("打开 session 失败: ${e.message}")
            }
        }
    }

    /** 在当前机器新建一个 session(默认进入当前项目目录)。 */
    fun newSession() {
        val machine = connection.activeMachine ?: return
        closeDrawer()
        viewModelScope.launch {
            runCatching { opencodeClient.createSession(machine.baseUrl) }
                .onSuccess { session ->
                    opencodeSessionId = session.id
                    sessionContext.clear()
                    _uiState.value = ChatUiState(sessionId = session.id)
                    refreshSessionTree()
                }
                .onFailure { e -> addError("新建 session 失败: ${e.message}") }
        }
    }

    private fun parseInstruction(arguments: String): String? = try {
        val obj = Json.parseToJsonElement(arguments).jsonObject
        obj["instruction"]?.jsonPrimitive?.content
    } catch (_: Exception) { null }

    private fun updateSecretary(id: String, text: String, reasoning: String?, streaming: Boolean) {
        _uiState.update { st ->
            st.copy(items = st.items.map { if (it.id == id && it is ChatItem.Secretary) it.copy(text = text, reasoning = reasoning, streaming = streaming) else it })
        }
    }

    private fun updateTask(id: String, status: TaskStatus, detail: String?) {
        _uiState.update { st ->
            st.copy(items = st.items.map { if (it.id == id && it is ChatItem.Task) it.copy(status = status, detail = detail) else it })
        }
    }

    private fun addError(text: String) {
        _uiState.update { it.copy(items = it.items + ChatItem.Error(genId(), text)) }
    }

    private fun genId(): String = UUID.randomUUID().toString()

    /** 断开连接:取消进行中的秘书任务,清空会话上下文与状态。UI 由 ConnectionManager 驱动跳回连接页。 */
    fun disconnect() {
        secretaryJob?.cancel()
        secretaryJob = null
        opencodeSessionId = null
        sessionContext.clear()
        _uiState.value = ChatUiState()
    }
}
