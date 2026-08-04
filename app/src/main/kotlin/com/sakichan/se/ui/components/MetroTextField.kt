package com.sakichan.se.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroColors
import io.github.takahashirinta.kanesumi.core.theme.LocalMetroTypography
import io.github.takahashirinta.kanesumi.core.theme.MetroText

/**
 * Metro 风文本输入框。直角无圆角、无边框、无 M3 outline/filled 语义。
 * 底色 surfaceVariant,placeholder 走 onSurfaceVariant。
 *
 * 待贡献回 Kanesumi 的 kanesumi-controls;当前先放 app 内验证(BUILD.md §6.2)。
 */
@Composable
fun MetroTextField(
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    readOnly: Boolean = false,
    placeholder: String = "",
    singleLine: Boolean = false,
    maxLines: Int = if (singleLine) 1 else Int.MAX_VALUE,
    minLines: Int = 1,
    visualTransformation: VisualTransformation = VisualTransformation.None,
    textStyle: TextStyle = LocalMetroTypography.current.body,
    contentPadding: PaddingValues = PaddingValues(horizontal = 12.dp, vertical = 12.dp),
    containerColor: Color = LocalMetroColors.current.surfaceVariant,
    cursorColor: Color = LocalMetroColors.current.primary,
) {
    val colors = LocalMetroColors.current
    val mergedStyle = textStyle.copy(color = if (enabled) colors.onSurface else colors.onSurfaceVariant)

    Box(
        modifier = modifier
            .background(containerColor)
            .padding(contentPadding),
        contentAlignment = Alignment.CenterStart,
    ) {
        BasicTextField(
            value = value,
            onValueChange = onValueChange,
            modifier = Modifier.heightIn(min = 20.dp),
            enabled = enabled,
            readOnly = readOnly,
            textStyle = mergedStyle,
            singleLine = singleLine,
            maxLines = maxLines,
            minLines = minLines,
            visualTransformation = visualTransformation,
            cursorBrush = SolidColor(cursorColor),
            decorationBox = { inner ->
                if (value.isEmpty()) {
                    CompositionLocalProvider(LocalLayoutDirection provides LayoutDirection.Ltr) {
                        MetroText(
                            text = placeholder,
                            color = colors.onSurfaceVariant,
                            style = mergedStyle,
                        )
                    }
                }
                inner()
            },
        )
    }
}
