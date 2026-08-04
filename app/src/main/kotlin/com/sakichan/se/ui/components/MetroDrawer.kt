package com.sakichan.se.ui.components

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.takahashirinta.kanesumi.anim.sokuou.SokuouTweens
import io.github.takahashirinta.kanesumi.core.insets.metroSystemBarsPadding
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroColors
import kotlinx.coroutines.launch

/**
 * Metro 风左侧汉堡抽屉。直角、无圆角、无 elevation -- 取代 M3 ModalDrawer 的
 * tonal scrim 渐变 + 圆角抽屉面。
 *
 * 动画模型与 MetroBottomSheet 同构:单一 `progress: Animatable<Float>` (0=藏,1=显):
 * - 入场走 SokuouTweens.SheetAppear (300ms MetroCubic);
 * - 收起走 SokuouTweens.SheetDismiss (260ms FastOutSlowIn);
 * - scrim alpha = progress * scrimAlpha (默认 0.6),点击 scrim 收起;
 * - 抽屉面 graphicsLayer { translationX = -width * (1 - progress) } 从左侧滑入。
 *
 * 宽默认 280dp;系统栏 inset 由内部 metroSystemBarsPadding() 代管,内容侧不用管。
 * 待贡献回 Kanesumi kanesumi-controls;当前放 app 内验证(BUILD.md §6.2)。
 */
@Composable
fun MetroDrawer(
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    scrimAlpha: Float = 0.6f,
    drawerColor: Color = LocalMetroColors.current.surface,
    drawerWidth: Dp = 280.dp,
    content: @Composable ColumnScope.() -> Unit,
) {
    val progress = remember { Animatable(0f) }
    val coroutineScope = rememberCoroutineScope()
    var drawerWidthPx by remember { mutableStateOf(0f) }

    LaunchedEffect(Unit) {
        progress.animateTo(1f, SokuouTweens.SheetAppear)
    }

    fun animateDismiss() {
        coroutineScope.launch {
            progress.animateTo(0f, SokuouTweens.SheetDismiss)
            onDismiss()
        }
    }

    Box(modifier = modifier.fillMaxSize()) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .graphicsLayer { alpha = progress.value * scrimAlpha }
                .background(Color.Black)
                .clickable(
                    indication = null,
                    interactionSource = remember { MutableInteractionSource() },
                ) { animateDismiss() },
        )

        Column(
            modifier = Modifier
                .align(Alignment.CenterStart)
                .width(drawerWidth)
                .fillMaxHeight()
                .onSizeChanged { drawerWidthPx = it.width.toFloat() }
                .graphicsLayer { translationX = -drawerWidthPx * (1f - progress.value) }
                .background(drawerColor)
                .metroSystemBarsPadding(),
        ) {
            content()
        }
    }
}
