package com.sakichan.se.ui.chat

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Build
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ExitToApp
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Warning
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import io.github.takahashirinta.kanesumi.controls.MetroButton
import io.github.takahashirinta.kanesumi.controls.MetroIconButton
import io.github.takahashirinta.kanesumi.controls.MetroProgressIndicator
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroColors
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroTypography
import io.github.takahashirinta.kanesumi.core.theme.MetroIcon
import io.github.takahashirinta.kanesumi.core.theme.MetroText
import io.github.takahashirinta.kanesumi.structure.MetroAppBar
import io.github.takahashirinta.kanesumi.structure.MetroShell
import com.sakichan.se.ui.components.MetroChatInputBar
import kotlinx.coroutines.delay

@Composable
fun ChatScreen(
    state: ChatUiState,
    onInputChange: (String) -> Unit,
    onSend: () -> Unit,
    onReplyPermission: (approve: Boolean, always: Boolean) -> Unit,
    onOpenSettings: () -> Unit,
    onDisconnect: () -> Unit,
) {
    val colors = LocalMetroColors.current
    val listState = rememberLazyListState()

    // 新消息时自动滚到底
    LaunchedEffect(state.items.size) {
        if (state.items.isNotEmpty()) {
            delay(60)
            listState.animateScrollToItem(state.items.size - 1)
        }
    }

    MetroShell(
        bottomBar = {
            if (state.pendingPermission != null) {
                PermissionBar(
                    permission = state.pendingPermission!!,
                    onReply = onReplyPermission,
                )
            } else {
                MetroChatInputBar(
                    text = state.inputText,
                    onTextChange = onInputChange,
                    onSend = onSend,
                    enabled = !state.isLoading,
                )
            }
        },
    ) {
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            state = listState,
            contentPadding = PaddingValues(bottom = 16.dp),
        ) {
            item {
                MetroAppBar(
                    title = "Sakichan",
                    actions = {
                        MetroIconButton(onClick = onDisconnect) {
                            MetroIcon(
                                imageVector = Icons.Filled.ExitToApp,
                                contentDescription = "断开连接",
                                tint = colors.onSurfaceVariant,
                                sizeDp = 22.dp,
                            )
                        }
                        MetroIconButton(onClick = onOpenSettings) {
                            MetroIcon(
                                imageVector = Icons.Filled.Settings,
                                contentDescription = "设置",
                                tint = colors.onSurfaceVariant,
                                sizeDp = 22.dp,
                            )
                        }
                    },
                )
            }

            if (state.notConfigured) {
                item { ConfigHint() }
            }

            items(state.items, key = { it.id }) { item -> ChatItemRow(item) }
        }
    }
}

@Composable
private fun ChatItemRow(item: ChatItem) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    when (item) {
        is ChatItem.User -> MessageBubble(
            text = item.text,
            bg = colors.primary,
            contentColor = colors.onPrimary,
            alignEnd = true,
        )
        is ChatItem.Secretary -> Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 4.dp),
        ) {
            if (!item.reasoning.isNullOrBlank()) {
                MetroText(
                    text = item.reasoning,
                    color = colors.onSurfaceVariant,
                    style = typography.caption,
                    modifier = Modifier.padding(bottom = 6.dp),
                )
            }
            if (item.text.isNotBlank() || item.streaming) {
                MetroText(
                    text = item.text.ifBlank { if (item.streaming) "思考中…" else "" },
                    color = colors.onSurface,
                    style = typography.body,
                )
            }
            if (item.streaming) {
                Spacer(Modifier.padding(top = 6.dp))
                MetroProgressIndicator(sizeDp = 18.dp, strokeDp = 2.dp)
            }
        }
        is ChatItem.Task -> TaskRow(item)
        is ChatItem.Error -> MessageBubble(
            text = item.text,
            bg = Color(0xFF3A1A1A),
            contentColor = Color(0xFFFFB4B4),
            alignEnd = false,
        )
    }
}

@Composable
private fun MessageBubble(
    text: String,
    bg: Color,
    contentColor: Color,
    alignEnd: Boolean,
) {
    val padding = if (alignEnd) {
        Modifier.padding(start = 64.dp, end = 16.dp, top = 4.dp, bottom = 4.dp)
    } else {
        Modifier.padding(start = 16.dp, end = 64.dp, top = 4.dp, bottom = 4.dp)
    }
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 0.dp),
        contentAlignment = if (alignEnd) Alignment.CenterEnd else Alignment.CenterStart,
    ) {
        Box(
            modifier = Modifier
                .then(padding)
                .background(bg)
                .padding(horizontal = 12.dp, vertical = 8.dp),
        ) {
            MetroText(text = text, color = contentColor, style = LocalMetroTypography.current.body)
        }
    }
}

@Composable
private fun TaskRow(task: ChatItem.Task) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val (icon, tint) = when (task.status) {
            TaskStatus.RUNNING -> Icons.Filled.Build to colors.primary
            TaskStatus.TOOL -> Icons.Filled.Build to colors.onSurfaceVariant
            TaskStatus.AWAITING_PERMISSION -> Icons.Filled.Build to colors.primary
            TaskStatus.DONE -> Icons.Filled.Check to colors.primary
            TaskStatus.FAILED -> Icons.Filled.Warning to Color(0xFFFFB4B4)
        }
        MetroIcon(imageVector = icon, contentDescription = null, tint = tint, sizeDp = 18.dp)
        Spacer(Modifier.width(8.dp))
        Column(modifier = Modifier.weight(1f)) {
            MetroText(
                text = task.instruction,
                color = colors.onSurface,
                style = typography.bodyMedium,
                maxLines = 2,
            )
            if (task.detail != null) {
                MetroText(
                    text = task.detail,
                    color = colors.onSurfaceVariant,
                    style = typography.caption,
                    maxLines = 3,
                )
            }
        }
        if (task.status == TaskStatus.RUNNING || task.status == TaskStatus.TOOL) {
            MetroProgressIndicator(sizeDp = 16.dp, strokeDp = 2.dp)
        }
    }
}

@Composable
private fun PermissionBar(
    permission: PendingPermission,
    onReply: (approve: Boolean, always: Boolean) -> Unit,
) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    val req = permission.request
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(colors.surfaceVariant)
            .padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
        MetroText(
            text = "权限请求: ${req.permission}",
            color = colors.onSurface,
            style = typography.bodyMedium,
        )
        if (req.patterns.isNotEmpty()) {
            MetroText(
                text = req.patterns.joinToString("\n"),
                color = colors.onSurfaceVariant,
                style = typography.caption,
                modifier = Modifier.padding(top = 2.dp),
            )
        }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(top = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            MetroButton(
                text = "允许",
                onClick = { onReply(true, false) },
                modifier = Modifier.weight(1f),
            )
            MetroButton(
                text = "总是允许",
                onClick = { onReply(true, true) },
                modifier = Modifier.weight(1f),
                containerColor = colors.surface,
                contentColor = colors.onSurface,
            )
            MetroIconButton(onClick = { onReply(false, false) }) {
                MetroIcon(
                    imageVector = Icons.Filled.Close,
                    contentDescription = "拒绝",
                    tint = Color(0xFFFFB4B4),
                    sizeDp = 22.dp,
                )
            }
        }
    }
}

@Composable
private fun ConfigHint() {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 12.dp)
            .background(colors.surfaceVariant)
            .padding(16.dp),
    ) {
        MetroText(
            text = "未配置",
            color = colors.onSurface,
            style = typography.titleMedium,
        )
        MetroText(
            text = "请在设置里填入 SenseNova API key 和 opencode server 地址。",
            color = colors.onSurfaceVariant,
            style = typography.caption,
            modifier = Modifier.padding(top = 4.dp),
        )
    }
}
