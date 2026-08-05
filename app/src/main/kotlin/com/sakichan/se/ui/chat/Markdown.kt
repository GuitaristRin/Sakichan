package com.sakichan.se.ui.chat

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle

/**
 * 轻量 Markdown 渲染:把 LLM 输出转成 AnnotatedString,支持:
 * - **加粗**      `**text**`
 * - `行内代码`   `` `text` ``
 * - # 标题       `# text` / `## text`
 * - 列表项       `- item` / `1. item`
 * - 代码块       ``` ``` text ``` ````
 * - 普通换行
 *
 * 不做完整 markdown(标题锚点/表格/链接等),够手机阅读即可。
 * 返回 null 表示无任何语法,调用方可直接显示原文。
 */
internal fun renderMarkdown(text: String, baseColor: Color): AnnotatedString {
    // 分块:代码块单独处理,其余按行
    val blocks = splitCodeBlocks(text)
    return buildAnnotatedString {
        blocks.forEachIndexed { bi, block ->
            if (block.isCode) {
                withStyle(SpanStyle(color = Color(0xFFCE9178))) {
                    append(block.text)
                }
            } else {
                block.text.lines().forEachIndexed { li, line ->
                    renderLine(line, baseColor)
                    if (li < block.text.lines().lastIndex || bi < blocks.lastIndex) {
                        append("\n")
                    }
                }
            }
        }
    }
}

private data class MdBlock(val text: String, val isCode: Boolean)

private fun splitCodeBlocks(text: String): List<MdBlock> {
    val lines = text.lines()
    val result = mutableListOf<MdBlock>()
    var i = 0
    val codeBuf = StringBuilder()
    var inCode = false
    while (i < lines.size) {
        val line = lines[i]
        val trimmed = line.trim()
        if (trimmed.startsWith("```")) {
            if (inCode) {
                result.add(MdBlock(codeBuf.toString().trimEnd('\n'), true))
                codeBuf.clear()
                inCode = false
            } else {
                inCode = true
            }
        } else if (inCode) {
            codeBuf.append(line).append('\n')
        } else {
            result.add(MdBlock(line, false))
        }
        i++
    }
    if (inCode && codeBuf.isNotEmpty()) {
        result.add(MdBlock(codeBuf.toString(), true))
    }
    return result
}

private fun AnnotatedString.Builder.renderLine(line: String, baseColor: Color) {
    val trimmed = line.trimStart()
    when {
        trimmed.startsWith("### ") || trimmed.startsWith("## ") || trimmed.startsWith("# ") -> {
            withStyle(SpanStyle(fontWeight = FontWeight.Bold)) {
                append(trimmed.substringAfter(' ').trim())
            }
        }
        trimmed.startsWith("- ") || trimmed.startsWith("* ") || trimmed.startsWith("• ") -> {
            append("• ")
            renderInline(trimmed.substringAfter(' ').trim(), baseColor)
        }
        Regex("""^\d+[.、)]\s""").containsMatchIn(trimmed) -> {
            append(trimmed)
        }
        else -> {
            renderInline(line, baseColor)
        }
    }
}

private fun AnnotatedString.Builder.renderInline(text: String, baseColor: Color) {
    // 支持 **bold** 和 `code`,交替识别
    var i = 0
    while (i < text.length) {
        when {
            text.startsWith("**", i) -> {
                val end = text.indexOf("**", i + 2)
                if (end > 0) {
                    withStyle(SpanStyle(fontWeight = FontWeight.Bold)) {
                        append(text.substring(i + 2, end))
                    }
                    i = end + 2
                } else {
                    append(text.substring(i))
                    i = text.length
                }
            }
            text.startsWith("`", i) -> {
                val end = text.indexOf('`', i + 1)
                if (end > 0) {
                    withStyle(SpanStyle(color = Color(0xFFCE9178))) {
                        append(text.substring(i + 1, end))
                    }
                    i = end + 1
                } else {
                    append(text.substring(i))
                    i = text.length
                }
            }
            else -> {
                val nextBold = text.indexOf("**", i)
                val nextCode = text.indexOf('`', i)
                val next = listOf(nextBold, nextCode).filter { it >= 0 }.minOrNull() ?: -1
                if (next < 0) {
                    append(text.substring(i))
                    i = text.length
                } else {
                    append(text.substring(i, next))
                    i = next
                }
            }
        }
    }
}
