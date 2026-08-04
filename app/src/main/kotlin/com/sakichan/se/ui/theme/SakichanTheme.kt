package com.sakichan.se.ui.theme

import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import io.github.takahashirinta.kanesumi.core.theme.MetroColors
import io.github.takahashirinta.kanesumi.core.theme.MetroTheme

private val SakichanColors = MetroColors(
    primary = Color(0xFF1C5035),
    onPrimary = Color(0xFFFFFFFF),
)

@Composable
fun SakichanTheme(
    content: @Composable () -> Unit,
) {
    MetroTheme(
        colors = SakichanColors,
        content = content,
    )
}
