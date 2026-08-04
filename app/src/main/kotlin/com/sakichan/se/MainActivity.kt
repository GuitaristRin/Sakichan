package com.sakichan.se

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.sakichan.se.connection.ConnectionManager
import com.sakichan.se.ui.chat.ChatScreen
import com.sakichan.se.ui.chat.ChatViewModel
import com.sakichan.se.ui.connection.ConnectionScreen
import com.sakichan.se.ui.connection.ConnectionViewModel
import com.sakichan.se.ui.settings.SettingsHost
import com.sakichan.se.ui.theme.SakichanTheme
import org.koin.compose.koinInject
import org.koin.compose.viewmodel.koinViewModel

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            SakichanTheme {
                AppRoot()
            }
        }
    }
}

private enum class Screen { CONNECTION, CHAT, SETTINGS }

@Composable
private fun AppRoot() {
    // 连接状态驱动顶层导航:未连接 -> 连接页;已连接 -> 聊天页
    val connection: ConnectionManager = koinInject()
    val connState by connection.state.collectAsStateWithLifecycle()
    var screen by remember { mutableStateOf(Screen.CONNECTION) }

    val connected = connState.activeMachine != null

    // 断开后强制回连接页
    if (!connected && screen == Screen.CHAT) {
        screen = Screen.CONNECTION
    }

    when (screen) {
        Screen.CONNECTION -> {
            val vm: ConnectionViewModel = koinViewModel()
            val state by vm.uiState.collectAsStateWithLifecycle()
            // 每次进入连接页都触发扫描(首次进入 / 断开后返回均生效)
            LaunchedEffect(Unit) { vm.startScan() }
            ConnectionScreen(
                state = state,
                onRefresh = vm::startScan,
                onSelectMachine = vm::connectMachine,
                onManualUrlChange = vm::onManualUrlChange,
                onConnectManual = vm::connectManual,
            )
            // 连接成功 -> 自动进聊天页
            if (connected) screen = Screen.CHAT
        }

        Screen.CHAT -> {
            val vm: ChatViewModel = koinViewModel()
            val state by vm.uiState.collectAsStateWithLifecycle()
            ChatScreen(
                state = state,
                onInputChange = vm::onInputChange,
                onSend = vm::send,
                onReplyPermission = vm::replyPermission,
                onOpenSettings = { screen = Screen.SETTINGS },
                onDisconnect = {
                    vm.disconnect()
                    connection.disconnect()
                },
                onOpenDrawer = vm::openDrawer,
                onCloseDrawer = vm::closeDrawer,
                onOpenSession = vm::openSession,
                onNewSession = vm::newSession,
            )
        }

        Screen.SETTINGS -> SettingsHost(onBack = { screen = if (connected) Screen.CHAT else Screen.CONNECTION })
    }
}
