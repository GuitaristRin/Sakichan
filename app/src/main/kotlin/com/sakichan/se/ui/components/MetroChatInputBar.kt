package com.sakichan.se.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.takahashirinta.kanesumi.controls.MetroIconButton
import io.github.takahashirinta.kanesumi.core.insets.metroNavigationBarsPadding
import io.github.takahashirinta.kanesumi.core.insets.rememberBottomStackReservation
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroColors
import io.github.takahashirinta.kanesumi.core.theme.MetroIcon

/**
 * 底部输入栏。挂 MetroBottomStack 自适应键盘高度,读 navigationBars 安全区。
 * 直角、无圆角、与 MetroShell bottomBar overlay 对齐。
 *
 * 待贡献回 Kanesumi;当前放 app 内验证(BUILD.md §6.2)。
 */
@Composable
fun MetroChatInputBar(
    text: String,
    onTextChange: (String) -> Unit,
    onSend: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    placeholder: String = "对秘书说点什么…",
    measuredHeightDp: androidx.compose.ui.unit.Dp = 72.dp,
) {
    val colors = LocalMetroColors.current
    // 登记底部高度,让内容侧 bottomOverlayPadding() 自动留白
    rememberBottomStackReservation(key = "sakichan.inputBar", heightDp = measuredHeightDp)

    Row(
        modifier = modifier
            .fillMaxWidth()
            .background(colors.surface)
            .metroNavigationBarsPadding()
            .padding(horizontal = 8.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        MetroTextField(
            value = text,
            onValueChange = onTextChange,
            modifier = Modifier
                .weight(1f)
                .heightIn(min = 40.dp),
            placeholder = placeholder,
            enabled = enabled,
            singleLine = false,
            maxLines = 5,
        )
        Spacer(Modifier.width(4.dp))
        MetroIconButton(onClick = onSend, enabled = enabled && text.isNotBlank()) {
            MetroIcon(
                imageVector = Icons.AutoMirrored.Filled.Send,
                contentDescription = "发送",
                tint = if (enabled && text.isNotBlank()) colors.primary else colors.onSurfaceVariant,
                sizeDp = 22.dp,
            )
        }
    }
}
