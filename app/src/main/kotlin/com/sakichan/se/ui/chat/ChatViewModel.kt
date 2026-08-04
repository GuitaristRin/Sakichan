package com.sakichan.se.ui.chat

import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.sakichan.se.connection.ConnectionManager
import com.sakichan.se.core.model.*
import com.sakichan.se.core.session.SessionContext
import com.sakichan.se.data.network.ChatApiClient
import com.sakichan.se.data.network.OpencodeClient
import com.sakichan.se.data.repository.AppConfigRepository
import com.sakichan.se.data.repository.ChatHistoryRepository
import com.sakichan.se.data.repository.SecretaryPrompt
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
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
    private val history: ChatHistoryRepository,
) : ViewModel() {

    private val _uiState = MutableStateFlow(ChatUiState())
    val uiState: StateFlow<ChatUiState> = _uiState.asStateFlow()

    private val sessionContext = SessionContext(
        sessionId = "local",
        modelId = "deepseek-v4-flash",
        systemPrompt = SecretaryPrompt.SYSTEM_PROMPT,
    )
    private var opencodeSessionId: String? = null
    private var opencodeSessionTitle: String? = null
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

        saveCurrentSession()
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
                            executeOpencodeTask(instruction, tc.id)
                        }
                    }
                }
            } else {
                _uiState.update { it.copy(isLoading = false) }
                saveCurrentSession()
            }
        }
    }

    /** 执行一个 opencode 任务:创建 session -> 发指令 -> 订阅事件流(+轮询兜底)-> 总结。 */
    private suspend fun executeOpencodeTask(instruction: String, toolCallId: String) {
        val url = activeBaseUrl() ?: return
        val directory = connection.activeDirectory
        val taskId = genId()
        val taskItem = ChatItem.Task(taskId, instruction, TaskStatus.RUNNING)
        _uiState.update { it.copy(items = it.items + taskItem) }

        val streamPreview = StringBuilder()
        var taskFailed: String? = null

        try {
            val sid = ensureOpencodeSession(url)
            _uiState.update { it.copy(sessionId = sid) }
            saveCurrentSession()

            opencodeClient.promptAsync(url, sid, OcMessageRequest(parts = listOf(OcTextPartInput(text = instruction))))

            // 事件流:实时增量(实测 v1 /event 端点),只做 UI 预览,不参与最终输出
            val eventsJob = viewModelScope.launch {
                runCatching {
                    opencodeClient.events(url, sid, directory ?: "").collect { ev ->
                        when (ev) {
                            is OcEvent.TextDelta -> {
                                streamPreview.append(ev.delta)
                                updateTask(taskId, TaskStatus.RUNNING, streamPreview.toString().takeLast(200))
                            }
                            is OcEvent.ReasoningDelta -> { /* opencode 思考块,暂不展示 */ }
                            is OcEvent.ToolCalled -> updateTask(taskId, TaskStatus.TOOL, ev.tool)
                            is OcEvent.ToolSuccess -> updateTask(taskId, TaskStatus.RUNNING, streamPreview.toString().takeLast(200))
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
                                taskFailed = ev.error
                                updateTask(taskId, TaskStatus.FAILED, ev.error)
                            }
                            else -> {}
                        }
                    }
                }
            }

            // 轮询兜底:事件流若不给 idle,靠 step-finish 判断完成
            var done = false
            var waited = 0
            var assistantText: String? = null
            while (!done && waited < 180) {
                delay(1000)
                waited++
                if (taskFailed != null) break
                runCatching {
                    opencodeClient.listMessages(url, sid)
                }.onSuccess { msgs ->
                    val assistantMsgs = msgs.filter { it.info.role == "assistant" }
                    if (assistantMsgs.isNotEmpty()) {
                        // 权威输出 = 消息里的完整 text part(不依赖事件流)
                        assistantText = assistantMsgs.joinToString("\n") { m ->
                            m.parts.filter { it.type == "text" }.joinToString("") { it.text ?: "" }
                        }.trim().ifBlank { null }
                        val preview = assistantText
                        if (!preview.isNullOrBlank()) {
                            updateTask(taskId, TaskStatus.RUNNING, preview.takeLast(200))
                        }
                        done = assistantMsgs.any { m ->
                            m.parts.any { it.type == "step-finish" }
                        }
                    }
                }
            }
            eventsJob.cancel()

            if (taskFailed != null) {
                updateTask(taskId, TaskStatus.FAILED, taskFailed)
                _uiState.update { it.copy(isLoading = false) }
                return
            }

            val finalOutput = assistantText ?: streamPreview.toString().trim()

            updateTask(taskId, TaskStatus.DONE, null)
            saveCurrentSession()

            // 把 opencode 输出作为 tool result 回灌秘书,让它总结
            val toolResultMsg = Message.tool(
                content = if (finalOutput.isBlank()) "任务执行完毕(无文本输出)" else finalOutput,
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
        saveCurrentSession()
    }

    private suspend fun ensureOpencodeSession(url: String): String {
        opencodeSessionId?.let { return it }
        val directory = connection.activeDirectory
        val session = opencodeClient.createSession(url, directory = directory)
        opencodeSessionId = session.id
        opencodeSessionTitle = session.title
        return session.id
    }

    /** 把当前会话(消息 + 展示项 + 标题)持久化到本地,断网可恢复。 */
    private fun saveCurrentSession() {
        val machine = connection.activeMachine ?: return
        val sid = opencodeSessionId ?: return
        viewModelScope.launch {
            runCatching {
                val items = _uiState.value.items.mapNotNull { item ->
                    when (item) {
                        is ChatItem.User -> PersistedChatItem(item.id, "user", item.text)
                        is ChatItem.Secretary -> PersistedChatItem(item.id, "secretary", item.text, item.reasoning)
                        else -> null
                    }
                }
                history.saveChat(
                    machineId = machine.id,
                    chat = PersistedChatSession(
                        sessionId = sid,
                        title = opencodeSessionTitle,
                        lastActiveAt = System.currentTimeMillis(),
                        messages = sessionContext.messages(),
                        items = items,
                    ),
                )
            }.onFailure { e -> Log.w("ChatPersistence", "save failed: ${e.message}") }
        }
    }

    /** 从本地缓存恢复展示项:仅保留 user/secretary,任务与错误行是瞬时状态不恢复。 */
    private fun restoreItems(chat: PersistedChatSession): List<ChatItem> =
        chat.items.map { item ->
            when (item.type) {
                "secretary" -> ChatItem.Secretary(item.id, item.text, item.reasoning, streaming = false)
                else -> ChatItem.User(item.id, item.text)
            }
        }

    // ===== 抽屉 / session 树管理 =====

    fun openDrawer() {
        _uiState.update { it.copy(drawerOpen = true) }
        refreshSessionTree()
    }

    fun closeDrawer() {
        _uiState.update { it.copy(drawerOpen = false) }
    }

    /** 拉取当前机器的「机器-项目-session」树,供抽屉展示。成功后缓存元数据,失败回退本地缓存。 */
    fun refreshSessionTree() {
        val machine = connection.activeMachine ?: return
        _uiState.update { it.copy(treeLoading = true) }
        viewModelScope.launch {
            runCatching { opencodeClient.buildSessionTree(machine) }
                .onSuccess { tree ->
                    _uiState.update { it.copy(sessionTree = tree, treeLoading = false) }
                    cacheTreeMeta(machine, tree)
                }
                .onFailure { _ ->
                    // 服务器不可达:用本地缓存重建树
                    val cached = runCatching { history.listSessions(machine.id) }.getOrDefault(emptyList())
                    val fallback = SessionTree(
                        machine = machine,
                        projects = cached.groupBy { it.projectID ?: "" }.map { (pid, metas) ->
                            ProjectNode(
                                project = OcProject(id = pid, name = pid.ifBlank { "本地缓存" }),
                                sessions = metas.map { OcSession(id = it.sessionId, title = it.title, projectID = it.projectID) },
                            )
                        },
                    )
                    _uiState.update { it.copy(sessionTree = fallback, treeLoading = false) }
                }
        }
    }

    /** 抽屉拉树成功后,把 session 元数据写入本地索引,供断网时回退。 */
    private fun cacheTreeMeta(machine: Machine, tree: SessionTree) {
        viewModelScope.launch {
            runCatching {
                val metas = tree.projects.flatMap { p ->
                    p.sessions.map { s ->
                        PersistedSessionMeta(
                            sessionId = s.id,
                            projectID = s.projectID ?: p.project.id,
                            title = s.title,
                            lastActiveAt = s.time?.created ?: System.currentTimeMillis(),
                        )
                    }
                }
                history.updateIndex(machine.id, metas)
            }
        }
    }

    /** 切换到一个已存在的 session:重建上下文,清空当前聊天区,并尝试从本地缓存恢复历史。 */
    fun openSession(sessionId: String) {
        val machine = connection.activeMachine ?: return
        closeDrawer()
        viewModelScope.launch {
            runCatching {
                opencodeClient.openSessionContext(machine, sessionId, SecretaryPrompt.SYSTEM_PROMPT)
            }.onSuccess { ctx ->
                sessionContext.loadMessages(emptyList())
                opencodeSessionId = sessionId
                opencodeSessionTitle = ctx.title
                // 从本地缓存恢复聊天历史(仅当本机曾聊过这个 session)
                val cached = runCatching { history.loadChat(machine.id, sessionId) }.getOrNull()
                if (cached != null) {
                    opencodeSessionTitle = cached.title ?: ctx.title
                    sessionContext.loadMessages(cached.messages)
                    _uiState.value = ChatUiState(sessionId = sessionId, items = restoreItems(cached))
                } else {
                    _uiState.value = ChatUiState(sessionId = sessionId)
                }
            }.onFailure { e ->
                // 服务器不可达:回退本地缓存
                val cached = runCatching { history.loadChat(machine.id, sessionId) }.getOrNull()
                if (cached != null) {
                    opencodeSessionId = sessionId
                    opencodeSessionTitle = cached.title
                    sessionContext.loadMessages(cached.messages)
                    _uiState.value = ChatUiState(sessionId = sessionId, items = restoreItems(cached))
                } else {
                    addError("打开 session 失败: ${e.message}")
                }
            }
        }
    }

    /** 在当前机器新建一个 session(默认进入当前项目目录)。 */
    fun newSession() {
        val machine = connection.activeMachine ?: return
        closeDrawer()
        viewModelScope.launch {
            runCatching { opencodeClient.createSession(machine.baseUrl, directory = connection.activeDirectory) }
                .onSuccess { session ->
                    opencodeSessionId = session.id
                    opencodeSessionTitle = session.title
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
        saveCurrentSession()
        opencodeSessionId = null
        opencodeSessionTitle = null
        sessionContext.clear()
        _uiState.value = ChatUiState()
    }

    /** 重连后恢复本机最近的活跃 session(本地缓存优先,离线也能看历史)。 */
    fun restoreLastSession() {
        val machine = connection.activeMachine ?: return
        if (_uiState.value.sessionId != null) return
        viewModelScope.launch {
            val metas = runCatching { history.listSessions(machine.id) }.getOrDefault(emptyList())
            val last = metas.maxByOrNull { it.lastActiveAt } ?: return@launch
            // 直接走缓存恢复;服务器详情在抽屉/后续操作中再对齐
            val cached = runCatching { history.loadChat(machine.id, last.sessionId) }.getOrNull()
            if (cached != null) {
                opencodeSessionId = cached.sessionId
                opencodeSessionTitle = cached.title
                sessionContext.loadMessages(cached.messages)
                _uiState.value = ChatUiState(sessionId = cached.sessionId, items = restoreItems(cached))
            }
        }
    }

    /** 删除某 session 的本地缓存(不删服务器上的)。若删的是当前会话,回到空白页。 */
    fun deleteSessionCache(sessionId: String) {
        val machine = connection.activeMachine ?: return
        viewModelScope.launch {
            runCatching { history.deleteChat(machine.id, sessionId) }
                .onSuccess {
                    if (opencodeSessionId == sessionId) {
                        opencodeSessionId = null
                        opencodeSessionTitle = null
                        sessionContext.clear()
                        _uiState.value = ChatUiState()
                    }
                    refreshSessionTree()
                }
        }
    }
}
