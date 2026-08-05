package com.sakichan.se.ui.connection

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.sakichan.se.connection.ConnectionManager
import com.sakichan.se.core.model.Machine
import com.sakichan.se.data.discovery.DiscoveryService
import com.sakichan.se.data.network.OpencodeClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.launch

/**
 * 连接页 ViewModel:扫描附近 opencode 机器,选一台连(健康检查+拉项目列表),
 * 或手动输入地址连。连接成功经 [ConnectionManager] 持久化活跃机器,触发 UI 跳聊天页。
 */
class ConnectionViewModel(
    private val discovery: DiscoveryService,
    private val opencode: OpencodeClient,
    private val connection: ConnectionManager,
) : ViewModel() {

    private val _uiState = MutableStateFlow(ConnectionUiState(scanning = false))
    val uiState: StateFlow<ConnectionUiState> = _uiState.asStateFlow()

    private var scanJob: kotlinx.coroutines.Job? = null

    /** 连接页进入时触发扫描。从聊天页断开回来也会再次调用(VM 存活,init 不再执行)。 */
    fun startScan() {
        scanJob?.cancel()
        _uiState.update { it.copy(scanning = true, machines = emptyList(), error = null) }
        scanJob = viewModelScope.launch {
            discovery.scan()
                .catch { e -> _uiState.update { it.copy(scanning = false, error = "扫描失败: ${e.message}") } }
                .collect { machines ->
                    _uiState.update { it.copy(scanning = false, machines = machines) }
                }
        }
    }

    fun onManualUrlChange(url: String) {
        _uiState.update { it.copy(manualUrl = url) }
    }

    /** 选扫描到的机器连接。 */
    fun connectMachine(machine: Machine) {
        doConnect(machine)
    }

    /** 手动输入地址连接。 */
    fun connectManual() {
        val url = _uiState.value.manualUrl.trim().trimEnd('/')
        if (url.isEmpty()) return
        val id = DiscoveryService.machineId(url)
        val machine = Machine(
            id = id,
            baseUrl = url,
            name = "opencode",
            host = url.removePrefix("http://").removePrefix("https://").substringBefore(':'),
            port = url.substringAfterLast(':').toIntOrNull() ?: 4096,
            source = Machine.Source.MANUAL,
        )
        doConnect(machine)
    }

    private fun doConnect(machine: Machine) {
        _uiState.update { it.copy(connecting = true, error = null) }
        connection.setConnecting()
        viewModelScope.launch {
            try {
                val health = opencode.health(machine.baseUrl)
                val projects = opencode.listProjects(machine.baseUrl)
                // 当前工作目录 = 首个非根的有效项目 worktree;不再单独调 /project/current(省一次网络往返,减少失败面)
                val directory = projects.firstOrNull { !it.worktree.isNullOrBlank() && it.worktree != "/" }?.worktree
                val resolved = machine.copy(version = health.version)
                connection.connect(resolved, projects, directory)
                _uiState.update { it.copy(connecting = false) }
            } catch (e: Exception) {
                val msg = "连接失败: ${e.message ?: e.javaClass.simpleName}"
                _uiState.update { it.copy(connecting = false, error = msg) }
                connection.fail(msg)
            }
        }
    }
}

private fun <T> MutableStateFlow<T>.update(block: (T) -> T) {
    value = block(value)
}
