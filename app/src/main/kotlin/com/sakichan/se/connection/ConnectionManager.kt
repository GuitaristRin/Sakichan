package com.sakichan.se.connection

import com.sakichan.se.core.model.Machine
import com.sakichan.se.core.model.OcProject
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * 运行时连接状态。app 启动时 [activeMachine] = null -> 显示连接页;
 * 连接成功后 [activeMachine] 填充 -> 跳聊天页。断开 -> 清空回连接页。
 *
 * session 树架构(BUILD.md §5 + 用户需求):「机器 -> 项目目录 -> session」。
 * 一台机器有多个 project(工作目录),每个 project 下若干 session。
 * 当前 MVP 连接时拉取 projectList 缓存;session 列表按需在 ChatViewModel 取。
 */
data class ConnectionState(
    val activeMachine: Machine? = null,
    val projects: List<OcProject> = emptyList(),
    val currentDirectory: String? = null,
    val connecting: Boolean = false,
    val error: String? = null,
)

class ConnectionManager {
    private val _state = MutableStateFlow(ConnectionState())
    val state: StateFlow<ConnectionState> = _state.asStateFlow()

    val activeMachine: Machine? get() = _state.value.activeMachine
    val activeBaseUrl: String? get() = _state.value.activeMachine?.baseUrl

    /** 当前机器的活跃工作目录(建 session / 订阅 /event 流用)。 */
    val activeDirectory: String? get() = _state.value.currentDirectory

    /** 标记正在连接(健康检查 + 拉项目列表进行中)。 */
    fun setConnecting() {
        _state.value = _state.value.copy(connecting = true, error = null)
    }

    /** 连接成功:记录机器 + 其项目列表 + 当前工作目录。 */
    fun connect(machine: Machine, projects: List<OcProject>, currentDirectory: String? = null) {
        _state.value = ConnectionState(
            activeMachine = machine.copy(reachable = true),
            projects = projects,
            currentDirectory = currentDirectory
                ?: projects.firstOrNull { !it.worktree.isNullOrBlank() && it.worktree != "/" }?.worktree,
            connecting = false,
            error = null,
        )
    }

    fun fail(error: String) {
        _state.value = _state.value.copy(connecting = false, error = error)
    }

    /** 断开:清空活跃机器,回到连接页。session 上下文由 ChatViewModel 自行清。 */
    fun disconnect() {
        _state.value = ConnectionState()
    }
}
