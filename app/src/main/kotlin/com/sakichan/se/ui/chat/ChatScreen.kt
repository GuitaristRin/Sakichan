package com.sakichan.se.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.BasicText
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Build
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ExitToApp
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Warning
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.takahashirinta.kanesumi.controls.MetroButton
import io.github.takahashirinta.kanesumi.controls.MetroChatInputBar
import io.github.takahashirinta.kanesumi.controls.MetroDivider
import io.github.takahashirinta.kanesumi.controls.MetroDrawer
import io.github.takahashirinta.kanesumi.controls.MetroIconButton
import io.github.takahashirinta.kanesumi.controls.MetroProgressIndicator
import io.github.takahashirinta.kanesumi.core.insets.bottomOverlayPadding
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroColors
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroTypography
import io.github.takahashirinta.kanesumi.core.theme.MetroIcon
import io.github.takahashirinta.kanesumi.core.theme.MetroText
import io.github.takahashirinta.kanesumi.structure.MetroAppBar
import io.github.takahashirinta.kanesumi.structure.MetroShell
import kotlinx.coroutines.delay

@Composable
fun ChatScreen(
    state: ChatUiState,
    onInputChange: (String) -> Unit,
    onSend: () -> Unit,
    onAbort: () -> Unit,
    onReplyPermission: (approve: Boolean, always: Boolean) -> Unit,
    onApproveProposal: (ChatItem.Proposal) -> Unit,
    onRejectProposal: (ChatItem.Proposal) -> Unit,
    onOpenSettings: () -> Unit,
    onDisconnect: () -> Unit,
    onOpenDrawer: () -> Unit,
    onCloseDrawer: () -> Unit,
    onOpenSession: (String) -> Unit,
    onNewSession: () -> Unit,
    onDeleteSession: (String) -> Unit,
) {
    val colors = LocalMetroColors.current
    val listState = rememberLazyListState()

    // 新消息时自动滚到底;若用户正往上翻历史,不抢滚动
    LaunchedEffect(state.items.size) {
        if (state.items.isNotEmpty()) {
            delay(80)
            val lastVisible = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0
            val total = listState.layoutInfo.totalItemsCount
            val nearBottom = lastVisible >= total - 2
            if (nearBottom) {
                listState.animateScrollToItem(state.items.size - 1)
            }
        }
    }

    Box(modifier = Modifier.fillMaxSize()) {
        MetroShell(
            bottomBar = {
                if (state.pendingPermission != null) {
                    PermissionBar(
                        permission = state.pendingPermission!!,
                        onReply = onReplyPermission,
                    )
                } else {
                    val taskRunning = state.isLoading || state.ocStatus != null
                    MetroChatInputBar(
                        text = state.inputText,
                        onTextChange = onInputChange,
                        onSend = if (taskRunning) onAbort else onSend,
                        // opencode 运作中仍可输入(问进度),思考/总结轮中禁用
                        enabled = !state.isLoading || state.ocStatus != null,
                        sendIcon = if (taskRunning) Icons.Filled.Close else Icons.AutoMirrored.Filled.Send,
                        sendContentDescription = if (taskRunning) "停止" else "发送",
                        modifier = Modifier.imePadding(),
                    )
                }
            },
        ) {
            LazyColumn(
                modifier = Modifier
                    .fillMaxSize()
                    .imePadding(),
                state = listState,
                contentPadding = PaddingValues(bottom = bottomOverlayPadding().calculateBottomPadding() + 16.dp),
            ) {
                item {
                    MetroAppBar(
                        title = "Sakichan",
                        navigationIcon = {
                            MetroIconButton(onClick = onOpenDrawer) {
                                MetroIcon(
                                    imageVector = Icons.Filled.Menu,
                                    contentDescription = "session 列表",
                                    tint = colors.onSurface,
                                    sizeDp = 22.dp,
                                )
                            }
                        },
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

                item(key = "status-dots") {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(14.dp),
                    ) {
                        WorkDot(label = "Sakichan", active = state.isLoading)
                        WorkDot(label = "opencode", active = state.ocStatus != null)
                    }
                }

                items(state.items, key = { it.id }) { item ->
                    ChatItemRow(item, onApproveProposal, onRejectProposal)
                }

                // opencode 运作状态条:作为列表底部元素,不遮挡输入栏,不干扰点击
                state.ocStatus?.let { status ->
                    item(key = "oc-status") { OcStatusBar(status) }
                }
            }
        }

        if (state.drawerOpen) {
            SessionTreeDrawer(
                state = state,
                onDismiss = onCloseDrawer,
                onOpenSession = onOpenSession,
                onNewSession = onNewSession,
                onDeleteSession = onDeleteSession,
            )
        }
    }
}

/** opencode 运作状态条:底部固定,带脉冲动画,显示实时 delta。 */
@Composable
private fun OcStatusBar(status: OcStatus) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(colors.primary.copy(alpha = 0.12f))
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        MetroProgressIndicator(sizeDp = 16.dp, strokeDp = 2.dp)
        Spacer(Modifier.width(10.dp))
        Column(modifier = Modifier.weight(1f)) {
            MetroText(
                text = status.text,
                color = colors.primary,
                style = typography.caption,
                maxLines = 1,
            )
            status.lastDelta?.let {
                MetroText(
                    text = it.take(60),
                    color = colors.onSurfaceVariant,
                    style = typography.caption,
                    maxLines = 1,
                )
            }
        }
    }
}

// 工作指示灯:亮绿=工作中,灰=空闲
private val statusOn = Color(0xFF2ECC40)
private val statusOff = Color(0x55333333)

@Composable
private fun WorkDot(label: String, active: Boolean) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(
            modifier = Modifier
                .size(10.dp)
                .background(if (active) statusOn else statusOff, shape = CircleShape)
                .semantics { contentDescription = if (active) "$label 工作中" else "$label 空闲" },
        )
        Spacer(Modifier.width(6.dp))
        MetroText(
            text = label,
            color = if (active) colors.primary else colors.onSurfaceVariant,
            style = typography.caption,
        )
    }
}

@Composable
private fun ChatItemRow(
    item: ChatItem,
    onApproveProposal: (ChatItem.Proposal) -> Unit,
    onRejectProposal: (ChatItem.Proposal) -> Unit,
) {
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
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(colors.surfaceVariant, shape = androidx.compose.foundation.shape.RoundedCornerShape(0.dp))
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                ) {
                    if (item.text.isNotBlank()) {
                        BasicText(
                            text = renderMarkdown(item.text, colors.onSurface),
                            style = typography.body.copy(color = colors.onSurface),
                        )
                    } else if (item.streaming) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            MetroProgressIndicator(sizeDp = 16.dp, strokeDp = 2.dp)
                            Spacer(Modifier.width(8.dp))
                            MetroText(
                                text = "正在输入…",
                                color = colors.onSurfaceVariant,
                                style = typography.caption,
                            )
                        }
                    }
                }
            }
        }
        is ChatItem.Task -> TaskRow(item)
        is ChatItem.Proposal -> ProposalRow(item, onApproveProposal, onRejectProposal)
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
private fun ProposalRow(
    proposal: ChatItem.Proposal,
    onApprove: (ChatItem.Proposal) -> Unit,
    onReject: (ChatItem.Proposal) -> Unit,
) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 6.dp)
            .background(colors.surfaceVariant)
            .padding(horizontal = 14.dp, vertical = 12.dp),
    ) {
        MetroText(
            text = "执行计划",
            color = colors.primary,
            style = typography.bodyMedium,
        )
        if (proposal.plan.isNotBlank()) {
            MetroText(
                text = proposal.plan,
                color = colors.onSurface,
                style = typography.body,
                modifier = Modifier.padding(top = 6.dp),
            )
        } else {
            MetroText(
                text = proposal.instruction,
                color = colors.onSurfaceVariant,
                style = typography.caption,
                modifier = Modifier.padding(top = 6.dp),
            )
        }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(top = 10.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            MetroButton(
                text = "批准执行",
                onClick = { onApprove(proposal) },
                modifier = Modifier.weight(1f),
            )
            MetroButton(
                text = "先不要",
                onClick = { onReject(proposal) },
                modifier = Modifier.weight(1f),
                containerColor = colors.surface,
                contentColor = colors.onSurface,
            )
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

@Composable
private fun SessionTreeDrawer(
    state: ChatUiState,
    onDismiss: () -> Unit,
    onOpenSession: (String) -> Unit,
    onNewSession: () -> Unit,
    onDeleteSession: (String) -> Unit,
) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    val tree = state.sessionTree

    MetroDrawer(onDismiss = onDismiss) {
        Column(modifier = Modifier.fillMaxSize()) {
            // 顶栏:App 名 + 当前机器
            Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 16.dp)) {
                MetroText(
                    text = "Sakichan",
                    color = colors.onSurface,
                    style = typography.pageHeading,
                )
                tree?.machine?.let {
                    MetroText(
                        text = "${it.name} · ${it.host}:${it.port}",
                        color = colors.onSurfaceVariant,
                        style = typography.caption,
                        modifier = Modifier.padding(top = 2.dp),
                        maxLines = 1,
                    )
                }
            }

            MetroDivider()

            // 新建 session:整块按钮,直角底色区分
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(colors.surfaceVariant)
                    .clickable(onClick = onNewSession)
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                MetroIcon(
                    imageVector = Icons.Filled.Add,
                    contentDescription = null,
                    tint = colors.primary,
                    sizeDp = 20.dp,
                )
                Spacer(Modifier.width(8.dp))
                MetroText(text = "新建 session", color = colors.onSurface, style = typography.bodyMedium)
            }

            MetroDivider()

            if (state.treeLoading && tree == null) {
                Box(
                    modifier = Modifier.fillMaxWidth().padding(16.dp),
                    contentAlignment = Alignment.Center,
                ) { MetroProgressIndicator(sizeDp = 20.dp, strokeDp = 2.dp) }
            }

            // 项目 -> session 树
            val projects = tree?.projects.orEmpty()
            if (projects.isEmpty() && !state.treeLoading) {
                MetroText(
                    text = "暂无项目 / session",
                    color = colors.onSurfaceVariant,
                    style = typography.caption,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }

            LazyColumn(modifier = Modifier.weight(1f)) {
                projects.forEach { project ->
                    item(key = "p-${project.project.id}") {
                        ProjectHeader(
                            name = project.project.name ?: project.project.worktree?.substringAfterLast('/')
                                ?: project.project.id,
                            path = project.project.worktree,
                            sessionCount = project.sessions.size,
                        )
                    }
                    project.sessions.forEach { session ->
                        item(key = session.id) {
                            SessionRow(
                                sessionId = session.id,
                                title = session.title
                                    ?: "session · ${session.id.takeLast(6)}",
                                active = state.sessionId == session.id,
                                onClick = { onOpenSession(session.id) },
                                onDelete = { onDeleteSession(session.id) },
                            )
                        }
                    }
                    if (project.sessions.isEmpty()) {
                        item(key = "empty-${project.project.id}") {
                            MetroText(
                                text = "  无 session",
                                color = colors.onSurfaceVariant,
                                style = typography.caption,
                                modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ProjectHeader(name: String, path: String?, sessionCount: Int) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(colors.surfaceVariant)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            MetroText(text = name, color = colors.onSurface, style = typography.bodyMedium)
            if (path != null && path != name) {
                MetroText(
                    text = path,
                    color = colors.onSurfaceVariant,
                    style = typography.caption,
                    maxLines = 1,
                )
            }
        }
        MetroText(
            text = "$sessionCount",
            color = colors.onSurfaceVariant,
            style = typography.label,
        )
    }
}

@Composable
private fun SessionRow(
    sessionId: String,
    title: String,
    active: Boolean,
    onClick: () -> Unit,
    onDelete: () -> Unit,
) {
    val colors = LocalMetroColors.current
    val typography = LocalMetroTypography.current
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(start = 16.dp, end = 8.dp, top = 10.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        MetroText(
            text = if (active) "● " else "· ",
            color = if (active) colors.primary else colors.onSurfaceVariant,
            style = typography.body,
        )
        MetroText(
            text = title,
            color = if (active) colors.primary else colors.onSurface,
            style = typography.body,
            maxLines = 1,
            modifier = Modifier.weight(1f),
        )
        MetroIconButton(onClick = onDelete) {
            MetroIcon(
                imageVector = Icons.Filled.Close,
                contentDescription = "删除本地缓存",
                tint = colors.onSurfaceVariant,
                sizeDp = 16.dp,
            )
        }
    }
}
