package com.sakichan.se.ui.chat

import android.content.Context
import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.sakichan.se.connection.ConnectionManager
import com.sakichan.se.core.agent.ThinkingAgent
import com.sakichan.se.core.model.*
import com.sakichan.se.core.session.SessionContext
import com.sakichan.se.data.network.ChatApiClient
import com.sakichan.se.data.network.OpencodeClient
import com.sakichan.se.data.repository.AppConfigRepository
import com.sakichan.se.data.repository.ChatHistoryRepository
import com.sakichan.se.data.repository.PendingCompletionStore
import com.sakichan.se.data.repository.SecretaryPrompt
import com.sakichan.se.service.TaskMonitorService
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.launch
import java.util.UUID

/**
 * 双层秘书编排(BUILD.md §4 重构)。
 *
 * 用户消息 -> [ThinkingAgent] 自驱思考(只读 skill 收集上下文)-> 终态动作:
 * - [ThinkingAgent.Thought.StartTask] 表层确认 -> 启动 opencode -> 底部状态条 -> 总结
 * - [ThinkingAgent.Thought.Reply]     表层直接展示回复
 *
 * opencode 运作时:底部状态条动画;用户随时可问进度,秘书基于当前状态回复。
 * 秘书可主动发消息(oc 完成 / 需权限)。
 *
 * 稳健性:opencode 任务执行期间启动 [TaskMonitorService] 前台服务保活进程;
 * app 被杀后服务检测完成并持久化结果,用户重开 app 时补跑审查 + 总结。
 */
class ChatViewModel(
    private val chatApiClient: ChatApiClient,
    private val opencodeClient: OpencodeClient,
    private val config: AppConfigRepository,
    private val connection: ConnectionManager,
    private val history: ChatHistoryRepository,
    private val pendingStore: PendingCompletionStore,
    private val app: Context,
) : ViewModel() {

    private val _uiState = MutableStateFlow(ChatUiState())
    val uiState: StateFlow<ChatUiState> = _uiState.asStateFlow()

    /** 秘书对话上下文(思考层 LLM 的消息历史)。 */
    private val sessionContext = SessionContext(
        sessionId = "local",
        modelId = "deepseek-v4-flash",
        systemPrompt = SecretaryPrompt.THINKING_PROMPT,
    )
    private var sakichanSessionId: String? = null
    private var opencodeSessionId: String? = null
    private var opencodeSessionTitle: String? = null
    private var secretaryJob: Job? = null
    private var ocJob: Job? = null

    /** 思考层 agent。 */
    private val thinkingAgent = ThinkingAgent(chatApiClient, opencodeClient, config).also {
        it.baseUrlProvider = { connection.activeBaseUrl }
    }

    private fun activeBaseUrl(): String? = connection.activeBaseUrl

    fun onInputChange(text: String) {
        _uiState.update { it.copy(inputText = text) }
    }

    /** 用户发送消息:进入思考层。opencode 运作中也可发消息(问进度/打断)。 */
    fun send() {
        val text = _uiState.value.inputText.trim()
        if (text.isEmpty()) return
        if (activeBaseUrl() == null) {
            addError("未连接 opencode,请先到连接页选择机器")
            return
        }
        // opencode 正在运作:允许发消息(问进度),但不再叠加新任务输入
        val ocRunning = _uiState.value.ocStatus != null
        if (ocRunning) {
            _uiState.update { it.copy(inputText = "") }
            val userItem = ChatItem.User(id = genId(), text = text)
            _uiState.update { it.copy(items = it.items + userItem) }
            sessionContext.addUserMessage(text)
            runProgressTurn(text)
            return
        }
        if (_uiState.value.isLoading) return
        if (sakichanSessionId == null) {
            sakichanSessionId = genId()
            opencodeSessionId = null
        }
        _uiState.update { it.copy(inputText = "", isLoading = true, sessionId = sakichanSessionId) }

        val userItem = ChatItem.User(id = genId(), text = text)
        _uiState.update { it.copy(items = it.items + userItem) }
        sessionContext.addUserMessage(text)
        saveCurrentSession()

        runThinkingTurn()
    }

    /**
     * opencode 运作中,用户问进度/打断。把当前任务状态 + 用户问题喂给思考层,
     * 让其判断:汇报进度(reply feedback) / 继续等 / 需要终止。
     */
    private fun runProgressTurn(userText: String) {
        secretaryJob?.cancel()
        secretaryJob = viewModelScope.launch {
            if (config.getModelConfig(config.getModelId()).apiKey.isBlank()) {
                addError("未配置 API key")
                return@launch
            }
            val status = _uiState.value.ocStatus
            val progressPrompt = "用户在你正在执行任务时发来一条消息。任务状态:${status?.text ?: "进行中"}。请用 reply(feedback) 简洁汇报当前进展;不要调 skill、不要 start_task。"
            val thought = runCatching {
                thinkingAgent.think(
                    context = sessionContext.messages().takeLast(10),
                    systemPrompt = progressPrompt,
                    maxIterations = 3,
                )
            }.getOrElse {
                ThinkingAgent.Thought.Reply("feedback", "稍等,我看下现在的进度。")
            }
            when (thought) {
                is ThinkingAgent.Thought.Reply -> {
                    sessionContext.addAssistantMessage(thought.message)
                    addSecretary(thought.message)
                    saveCurrentSession()
                }
                else -> {
                    // 思考层没产出:兜底一句,避免沉默
                    val fallback = "稍等,任务还在进行。"
                    sessionContext.addAssistantMessage(fallback)
                    addSecretary(fallback)
                    saveCurrentSession()
                }
            }
        }
    }

    /** 用户对权限请求的回复。 */
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

    /** 中止当前任务:取消思考轮 + opencode 轮询,并通知 serve 端 abort。 */
    fun abort() {
        val sid = opencodeSessionId
        val url = activeBaseUrl()
        secretaryJob?.cancel()
        secretaryJob = null
        ocJob?.cancel()
        ocJob = null
        TaskMonitorService.stop(app)
        pendingStore.clear()
        _uiState.update { it.copy(isLoading = false, ocStatus = null) }
        if (sid != null && url != null) {
            viewModelScope.launch {
                runCatching { opencodeClient.abort(url, sid) }
                // 更新任务状态为失败(中止)
                _uiState.update { st ->
                    st.copy(items = st.items.map {
                        if (it is ChatItem.Task && (it.status == TaskStatus.RUNNING || it.status == TaskStatus.TOOL || it.status == TaskStatus.AWAITING_PERMISSION))
                            it.copy(status = TaskStatus.FAILED, detail = "已中止")
                        else it
                    })
                }
            }
        }
    }

    // ===== 思考层 =====

    /** 用户批准任务提案:移除卡片 -> 执行 opencode 任务。 */
    fun approveProposal(proposal: ChatItem.Proposal) {
        _uiState.update { st ->
            st.copy(items = st.items.filter { it.id != proposal.id })
        }
        viewModelScope.launch { executeOpencodeTask(proposal.instruction) }
    }

    /** 用户拒绝任务提案:移除卡片,回到输入态。 */
    fun rejectProposal(proposal: ChatItem.Proposal) {
        _uiState.update { st ->
            st.copy(items = st.items.filter { it.id != proposal.id })
        }
        val msg = "好的,这个先不做。需要我调整方案吗?"
        sessionContext.addAssistantMessage(msg)
        addSecretary(msg)
        saveCurrentSession()
    }

    private fun runThinkingTurn() {
        secretaryJob?.cancel()
        secretaryJob = viewModelScope.launch {
            if (config.getModelConfig(config.getModelId()).apiKey.isBlank()) {
                _uiState.update { it.copy(isLoading = false, notConfigured = true) }
                return@launch
            }

            // 立即显示"正在输入…"占位,让用户知道秘书在处理(微信式反馈)
            val pendingId = genId()
            _uiState.update {
                it.copy(items = it.items + ChatItem.Secretary(pendingId, "", null, streaming = true), isLoading = true)
            }

            val thought = runCatching {
                thinkingAgent.think(
                    context = sessionContext.messages(),
                    callbacks = object : ThinkingAgent.Callbacks {
                        override fun onReply(kind: String, message: String) {
                            // 思考层的每次 reply 都实时呈现给用户,不让他干等
                            sessionContext.addAssistantMessage(message)
                            // 首个反馈替换占位,后续追加新消息
                            updateOrAppendSecretary(pendingId, message)
                            saveCurrentSession()
                        }
                    },
                )
            }.getOrElse {
                updateSecretaryText(pendingId, "抱歉,我这边出了点问题:${it.message}")
                _uiState.update { it.copy(isLoading = false) }
                return@launch
            }

            // 终态:summary 已由 callback 显示,start_task 展示提案等用户批准
            when (thought) {
                is ThinkingAgent.Thought.StartTask -> {
                    // 批准闸门(借鉴 sena):不直接执行,先展示 plan 让用户批准
                    sessionContext.addAssistantMessage(thought.confirm)
                    updateOrAppendSecretary(pendingId, thought.confirm)
                    _uiState.update {
                        it.copy(items = it.items + ChatItem.Proposal(
                            id = genId(),
                            instruction = thought.instruction,
                            confirm = thought.confirm,
                            plan = thought.plan,
                        ), isLoading = false)
                    }
                    saveCurrentSession()
                }
                is ThinkingAgent.Thought.Reply -> {
                    // summary 兜底(理论上已由 callback 显示)
                    val shown = _uiState.value.items.any { it is ChatItem.Secretary && it.text == thought.message && !it.streaming }
                    if (!shown) {
                        sessionContext.addAssistantMessage(thought.message)
                        updateOrAppendSecretary(pendingId, thought.message)
                    }
                    _uiState.update { it.copy(isLoading = false) }
                    saveCurrentSession()
                }
                else -> {
                    // End 带 message 则追加
                    if (thought is ThinkingAgent.Thought.End && !thought.message.isNullOrBlank()) {
                        sessionContext.addAssistantMessage(thought.message!!)
                        updateOrAppendSecretary(pendingId, thought.message!!)
                    }
                    _uiState.update { it.copy(isLoading = false) }
                    saveCurrentSession()
                }
            }
        }
    }

    /** 首个反馈替换"正在输入…"占位,后续反馈追加为独立消息。 */
    private fun updateOrAppendSecretary(pendingId: String, text: String) {
        _uiState.update { st ->
            // 仅当占位还是"正在输入"(streaming)时才替换;替换后换新 id,避免后续反馈又替换它
            val hasPending = st.items.any { it.id == pendingId && it is ChatItem.Secretary && it.streaming }
            if (hasPending) {
                st.copy(items = st.items.map {
                    if (it.id == pendingId && it is ChatItem.Secretary)
                        it.copy(id = genId(), text = text, streaming = false)
                    else it
                })
            } else {
                // 占位已被替换:追加新消息
                st.copy(items = st.items + ChatItem.Secretary(genId(), text, null, streaming = false))
            }
        }
    }

    private fun updateSecretaryText(id: String, text: String) {
        _uiState.update { st ->
            st.copy(items = st.items.map {
                if (it.id == id && it is ChatItem.Secretary) it.copy(text = text, streaming = false) else it
            })
        }
    }

    // ===== opencode 执行层 =====

    /** 执行 opencode 任务:创建 session -> 发指令 -> 订阅事件流(+轮询兜底)-> 总结。 */
    private suspend fun executeOpencodeTask(instruction: String) {
        val url = activeBaseUrl() ?: run { _uiState.update { it.copy(isLoading = false) } ; return }
        val directory = connection.activeDirectory
        val taskId = genId()
        val taskItem = ChatItem.Task(taskId, instruction, TaskStatus.RUNNING)
        // oc 运行中不阻塞输入栏(用户可随时问进度)
        _uiState.update {
            it.copy(items = it.items + taskItem, ocStatus = OcStatus("opencode 运作中…"), isLoading = false)
        }
        pendingStore.clear()  // 清掉上次的待处理记录

        val streamPreview = StringBuilder()
        var taskFailed: String? = null

        try {
            val sid = ensureOpencodeSession(url)
            // sessionId 保持本地会话 id;opencode session 绑定在 opencodeSessionId 字段
            saveCurrentSession()

            // 启动前台服务:保活进程 + 备份轮询(app 被杀时服务检测完成并持久化)
            sakichanSessionId?.let { sakiSid ->
                TaskMonitorService.start(app, url, sid, sakiSid, instruction)
            }

            opencodeClient.promptAsync(url, sid, OcMessageRequest(parts = listOf(OcTextPartInput(text = instruction))))

            // 事件流:实时增量(只做预览)
            val eventsJob = viewModelScope.launch {
                runCatching {
                    opencodeClient.events(url, sid, directory ?: "").collect { ev ->
                        when (ev) {
                            is OcEvent.TextDelta -> {
                                streamPreview.append(ev.delta)
                                updateTask(taskId, TaskStatus.RUNNING, streamPreview.toString().takeLast(200))
                                _uiState.update { it.copy(ocStatus = OcStatus("opencode 运作中…", ev.delta)) }
                            }
                            is OcEvent.ToolCalled -> {
                                updateTask(taskId, TaskStatus.TOOL, ev.tool)
                                _uiState.update { it.copy(ocStatus = OcStatus("工具: ${ev.tool}")) }
                            }
                            is OcEvent.ToolFailed -> updateTask(taskId, TaskStatus.RUNNING, "工具失败: ${ev.error}")
                            is OcEvent.PermissionAsked -> {
                                _uiState.update { it.copy(pendingPermission = PendingPermission(
                                    OcPermissionRequest(
                                        id = ev.requestID, sessionID = sid,
                                        permission = ev.permission, patterns = ev.patterns,
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
            ocJob = eventsJob

            // 轮询兜底:等 step-finish
            var done = false
            var waited = 0
            var assistantText: String? = null
            while (!done && waited < 180) {
                delay(1000)
                waited++
                if (taskFailed != null) break
                runCatching { opencodeClient.listMessages(url, sid) }.onSuccess { msgs ->
                    val assistantMsgs = msgs.filter { it.info.role == "assistant" }
                    if (assistantMsgs.isNotEmpty()) {
                        assistantText = assistantMsgs.joinToString("\n") { m ->
                            m.parts.filter { it.type == "text" }.joinToString("") { it.text ?: "" }
                        }.trim().ifBlank { null }
                        val preview = assistantText
                        if (!preview.isNullOrBlank()) {
                            updateTask(taskId, TaskStatus.RUNNING, preview.takeLast(200))
                        }
                        done = assistantMsgs.any { m -> m.parts.any { it.type == "step-finish" } }
                    }
                }
            }
            ocJob?.cancel()
            ocJob = null
            TaskMonitorService.stop(app)  // ViewModel 自己检测到完成,停服务
            pendingStore.clear()  // 清掉服务可能写入的待处理记录(ViewModel 已处理)

            _uiState.update { it.copy(ocStatus = null) }

            if (taskFailed != null) {
                updateTask(taskId, TaskStatus.FAILED, taskFailed)
                addSecretary("opencode 执行出错了:${taskFailed}")
                _uiState.update { it.copy(isLoading = false) }
                saveCurrentSession()
                return
            }

            val finalOutput = assistantText ?: streamPreview.toString().trim()
            updateTask(taskId, TaskStatus.DONE, null)

            // 把 opencode 输出回灌上下文,先跑对抗审查轮,再跑总结轮
            sessionContext.addMessage(Message.tool(
                content = if (finalOutput.isBlank()) "任务执行完毕(无文本输出)" else finalOutput,
                toolCallId = "oc_result",
            ))
            saveCurrentSession()
            val review = runReviewTurn(instruction, finalOutput)
            sessionContext.addMessage(Message.user("【审查层判定】\n$review"))
            saveCurrentSession()
            runSummaryTurn()
        } catch (e: Exception) {
            TaskMonitorService.stop(app)
            updateTask(taskId, TaskStatus.FAILED, e.message)
            _uiState.update { it.copy(isLoading = false, ocStatus = null) }
        }
    }

    /**
     * 对抗审查轮(借鉴 ringleader creator/critic 分离)。
     * opencode 跑完后,用挑剔视角判断任务是否真完成,产出判定文本供总结轮参考。
     * 用 roster.review 配置(理想异模型,当前同模型但对抗 prompt 已有 value)。
     */
    private suspend fun runReviewTurn(instruction: String, ocOutput: String): String {
        val roster = config.getRoster()
        val reviewModel = roster.review
        if (reviewModel.apiKey.isBlank()) return "判定: 未完成\n遗漏: 未配置审查模型\n说明: 跳过审查"

        val messages = buildList {
            add(Message.system(SecretaryPrompt.REVIEW_PROMPT))
            add(Message.user("用户任务:\n$instruction\n\nopencode 输出:\n${ocOutput.take(4000)}"))
        }

        var content = StringBuilder()
        runCatching {
            chatApiClient.streamChat(
                apiBase = reviewModel.apiBase,
                apiKey = reviewModel.apiKey,
                modelId = config.getModelId(),
                messages = messages,
                options = ChatOptions(extraParams = reviewModel.extraParams),
                isDeepSeek = true,
                tools = SecretaryPrompt.REVIEW_TOOLS,
            ).collect { ev ->
                when (ev) {
                    is PipelineEvent.Token -> content.append(ev.text)
                    is PipelineEvent.Done -> content = StringBuilder(ev.result.fullContent)
                    is PipelineEvent.Error -> content = StringBuilder("判定: 未完成\n遗漏: 审查出错\n说明: ${ev.text}")
                    else -> {}
                }
            }
        }.onFailure {
            content = StringBuilder("判定: 未完成\n遗漏: 审查异常\n说明: ${it.message}")
        }
        return content.toString().ifBlank { "判定: 未完成\n遗漏: 审查无输出\n说明: 跳过审查" }
    }

    /** 总结轮:基于 opencode 输出 + 审查层判定,产出面向用户的总结。 */
    private suspend fun runSummaryTurn() {
        val roster = config.getRoster()
        val summaryModel = roster.summary
        _uiState.update { it.copy(isLoading = true) }

        // 用总结 prompt 临时替换 system,跑一轮无工具 LLM
        val messages = buildList {
            add(Message.system(SecretaryPrompt.SUMMARY_PROMPT))
            addAll(sessionContext.messages().takeLast(20))  // 最近上下文足够总结(含审查判定)
        }

        var content = StringBuilder()
        chatApiClient.streamChat(
            apiBase = summaryModel.apiBase,
            apiKey = summaryModel.apiKey,
            modelId = config.getModelId(),
            messages = messages,
            options = ChatOptions(extraParams = summaryModel.extraParams),
            isDeepSeek = true,
            tools = SecretaryPrompt.SUMMARY_TOOLS,
        ).collect { ev ->
            when (ev) {
                is PipelineEvent.Token -> {
                    content.append(ev.text)
                    _uiState.update { st ->
                        // 流式更新最后一条 secretary 消息(或新建)
                        val items = st.items
                        val lastIdx = items.indexOfLast { it is ChatItem.Secretary && it.streaming }
                        if (lastIdx >= 0) {
                            st.copy(items = items.mapIndexed { i, item ->
                                if (i == lastIdx && item is ChatItem.Secretary)
                                    item.copy(text = content.toString(), streaming = true)
                                else item
                            })
                        } else {
                            st.copy(items = items + ChatItem.Secretary(genId(), content.toString(), null, true))
                        }
                    }
                }
                is PipelineEvent.Done -> {
                    sessionContext.addAssistantMessage(ev.result.fullContent)
                    finalizeLastSecretary(ev.result.fullContent)
                }
                is PipelineEvent.Error -> {
                    finalizeLastSecretary(content.toString())
                    addError(ev.text)
                }
                else -> {}
            }
        }
        _uiState.update { it.copy(isLoading = false) }
        saveCurrentSession()
    }

    // ===== 抽屉 / session 树 =====

    fun openDrawer() {
        _uiState.update { it.copy(drawerOpen = true) }
        refreshSessionTree()
    }

    fun closeDrawer() { _uiState.update { it.copy(drawerOpen = false) } }

    fun refreshSessionTree() {
        val machine = connection.activeMachine ?: return
        _uiState.update { it.copy(treeLoading = true) }
        viewModelScope.launch {
            // 抽屉 = Sakichan 本地会话列表(每个是独立对话线程),不按 opencode 项目树。
            // opencode session 是对话内部的绑定,不进抽屉层级。
            val sessions = runCatching { history.listSessions(machine.id) }.getOrDefault(emptyList())
            val tree = SessionTree(
                machine = machine,
                projects = listOf(
                    ProjectNode(
                        project = OcProject(id = "local", name = "我的会话"),
                        sessions = sessions.map { meta ->
                            OcSession(
                                id = meta.sessionId,
                                title = meta.title ?: "会话 · ${meta.sessionId.takeLast(6)}",
                                projectID = "local",
                            )
                        },
                    ),
                ),
            )
            _uiState.update { it.copy(sessionTree = tree, treeLoading = false) }
        }
    }

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

    /** 打开一个 Sakichan 本地会话(对话线程):从本地缓存恢复聊天,不依赖 opencode session。 */
    fun openSession(sessionId: String) {
        val machine = connection.activeMachine ?: return
        closeDrawer()
        TaskMonitorService.stop(app)
        viewModelScope.launch {
            val cached = runCatching { history.loadChat(machine.id, sessionId) }.getOrNull()
            if (cached != null) {
                sakichanSessionId = cached.sessionId
                opencodeSessionId = cached.opencodeSessionId
                opencodeSessionTitle = cached.title
                sessionContext.loadMessages(cached.messages)
                _uiState.value = ChatUiState(sessionId = sessionId, items = restoreItems(cached))
                checkPendingCompletion()
            } else {
                addError("本地会话不存在: $sessionId")
            }
        }
    }

    /** 新建一个 Sakichan 本地会话(对话线程)。opencode session 首次派活时才创建/绑定。 */
    fun newSession() {
        val machine = connection.activeMachine ?: return
        closeDrawer()
        sakichanSessionId = genId()
        opencodeSessionId = null
        opencodeSessionTitle = null
        sessionContext.clear()
        _uiState.value = ChatUiState(sessionId = sakichanSessionId)
        refreshSessionTree()
    }

    // ===== 持久化 =====

    private fun saveCurrentSession() {
        val machine = connection.activeMachine ?: return
        val sid = sakichanSessionId ?: return
        val msgs = sessionContext.messages()
        if (msgs.isEmpty() && opencodeSessionId == null) return
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
                        sessionId = sid, title = opencodeSessionTitle,
                        lastActiveAt = System.currentTimeMillis(),
                        messages = msgs, items = items,
                        opencodeSessionId = opencodeSessionId,
                    ),
                )
            }.onFailure { e -> Log.w("ChatPersistence", "save failed: ${e.message}") }
        }
    }

    private fun restoreItems(chat: PersistedChatSession): List<ChatItem> =
        chat.items.map { item ->
            when (item.type) {
                "secretary" -> ChatItem.Secretary(item.id, item.text, item.reasoning, streaming = false)
                else -> ChatItem.User(item.id, item.text)
            }
        }

    // ===== 辅助 =====

    private suspend fun ensureOpencodeSession(url: String): String {
        // 校验已有 session 是否还活着(PC 重启后旧 id 会 404)
        if (opencodeSessionId != null) {
            val valid = runCatching { opencodeClient.getSession(url, opencodeSessionId!!) }.isSuccess
            if (valid) return opencodeSessionId!!
            // 失效:清掉旧 id,新建
            Log.w("ChatViewModel", "opencode session ${opencodeSessionId} 已失效,重新创建")
            opencodeSessionId = null
        }
        val session = opencodeClient.createSession(url, directory = connection.activeDirectory)
        opencodeSessionId = session.id
        opencodeSessionTitle = session.title
        saveCurrentSession()
        return session.id
    }

    private fun addSecretary(text: String) {
        _uiState.update { it.copy(items = it.items + ChatItem.Secretary(genId(), text, null, streaming = false)) }
    }

    private fun finalizeLastSecretary(text: String) {
        _uiState.update { st ->
            st.copy(items = st.items.map { if (it is ChatItem.Secretary && it.streaming) it.copy(text = text, streaming = false) else it })
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

    fun disconnect() {
        secretaryJob?.cancel()
        secretaryJob = null
        TaskMonitorService.stop(app)
        saveCurrentSession()
        sakichanSessionId = null
        opencodeSessionId = null
        opencodeSessionTitle = null
        sessionContext.clear()
        _uiState.value = ChatUiState()
    }

    /**
     * 进入聊天页时恢复最近的 Sakichan 会话(本地对话线程)。无本地会话时开抽屉展示空列表。
     * 恢复后检查是否有 app 被杀期间完成的任务(pendingCompletion),有则补跑审查 + 总结。
     */
    fun restoreLastSession() {
        val machine = connection.activeMachine ?: return
        if (_uiState.value.sessionId != null) return
        viewModelScope.launch {
            val metas = runCatching { history.listSessions(machine.id) }.getOrDefault(emptyList())
            val last = metas.maxByOrNull { it.lastActiveAt }
            val cached = last?.let { runCatching { history.loadChat(machine.id, it.sessionId) }.getOrNull() }
            if (cached != null) {
                sakichanSessionId = cached.sessionId
                opencodeSessionId = cached.opencodeSessionId
                opencodeSessionTitle = cached.title
                sessionContext.loadMessages(cached.messages)
                _uiState.value = ChatUiState(sessionId = cached.sessionId, items = restoreItems(cached))
                // 恢复后检查:app 被杀期间是否有任务完成了但没跑审查 + 总结
                checkPendingCompletion()
            } else {
                // 无本地会话:开抽屉展示空会话列表,引导新建
                refreshSessionTree()
                _uiState.update { it.copy(drawerOpen = true) }
            }
        }
    }

    /**
     * 检查 [pendingStore] 中是否有待处理的任务完成记录。
     *
     * 三种来源:
     * 1. 前台服务在 app 被杀时检测到完成,存入了 store
     * 2. 服务也被杀:直接 poll opencode server 看 session 是否已完成
     *
     * 找到后:把输出回灌上下文,跑对抗审查 + 总结,清 store。
     */
    private suspend fun checkPendingCompletion() {
        val url = activeBaseUrl() ?: return
        val ocSid = opencodeSessionId ?: return
        val sakiSid = sakichanSessionId ?: return

        // 1. 检查 store(服务写入的)
        val pending = pendingStore.load()
        if (pending != null && pending.sakichanSessionId == sakiSid) {
            pendingStore.clear()
            recoverCompletion(pending, url)
            return
        }

        // 2. store 没有:可能服务也被杀了。直接 poll opencode server 检查 session 状态
        runCatching {
            val msgs = opencodeClient.listMessages(url, ocSid)
            val assistantMsgs = msgs.filter { it.info.role == "assistant" }
            val done = assistantMsgs.any { m -> m.parts.any { it.type == "step-finish" } }
            if (done) {
                val output = assistantMsgs.joinToString("\n") { m ->
                    m.parts.filter { it.type == "text" }.joinToString("") { it.text ?: "" }
                }.trim().ifBlank { "任务执行完毕(无文本输出)" }
                // 检查上下文里是否已经处理过(有 oc_result tool 消息说明已处理)
                val alreadyHandled = sessionContext.messages().any {
                    it.role == "tool" && it.toolCallId == "oc_result"
                }
                if (!alreadyHandled) {
                    recoverCompletion(PendingCompletion(
                        baseUrl = url,
                        opencodeSessionId = ocSid,
                        sakichanSessionId = sakiSid,
                        instruction = "（恢复的任务）",
                        output = output,
                    ), url)
                }
            }
        }
    }

    /** 从 [pending] 恢复:回灌输出 -> 审查 -> 总结。 */
    private suspend fun recoverCompletion(pending: PendingCompletion, url: String) {
        _uiState.update { it.copy(isLoading = true) }
        if (pending.failed) {
            addSecretary("之前的任务失败了:${pending.error ?: "未知错误"}")
            _uiState.update { it.copy(isLoading = false) }
            saveCurrentSession()
            return
        }
        addSecretary("你离开的时候任务完成了,我来看看结果。")
        sessionContext.addMessage(Message.tool(
            content = pending.output,
            toolCallId = "oc_result",
        ))
        saveCurrentSession()
        val review = runReviewTurn(pending.instruction, pending.output)
        sessionContext.addMessage(Message.user("【审查层判定】\n$review"))
        saveCurrentSession()
        runSummaryTurn()
    }

    fun deleteSessionCache(sessionId: String) {
        val machine = connection.activeMachine ?: return
        viewModelScope.launch {
            runCatching { history.deleteChat(machine.id, sessionId) }
                .onSuccess {
                    if (sakichanSessionId == sessionId) {
                        sakichanSessionId = null
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
