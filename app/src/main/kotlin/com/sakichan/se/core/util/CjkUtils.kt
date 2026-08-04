package com.sakichan.se.core.util

/**
 * CJK character detection and keyword extraction.
 * Port of Rust sakichan-core memory/retrieval.rs.
 */
object CjkUtils {

    private val CJK_RANGES = listOf(
        0x4E00..0x9FFF,    // CJK Unified Ideographs
        0x3400..0x4DBF,    // Extension A
        0x2F800..0x2FA1F   // Compatibility Ideographs Supplement
    )

    fun isCjk(c: Char): Boolean {
        val code = c.code
        return CJK_RANGES.any { code in it }
    }

    private val CJK_STOP_WORDS = setOf(
        "的", "了", "是", "在", "我", "有", "和", "就", "不", "人",
        "都", "一", "上", "也", "很", "到", "说", "一个", "没有",
        "看", "好", "自己", "这", "他", "她", "它", "们", "要", "去",
        "你", "会", "着"
    )

    private val ENGLISH_STOP_WORDS = setOf(
        "the", "a", "an", "is", "are", "was", "were", "be", "been",
        "being", "have", "has", "had", "do", "does", "did", "will",
        "would", "can", "could", "shall", "should", "may", "might",
        "must", "i", "you", "he", "she", "it", "we", "they", "me",
        "my", "your", "his", "her", "its", "our", "their", "this",
        "that", "these", "those", "and", "or", "but", "if", "because",
        "as", "of", "in", "on", "at", "by", "for", "with", "about",
        "between", "into", "through", "during", "to", "from", "not",
        "no", "nor", "so", "yet", "both", "either", "neither", "each",
        "every", "all", "any", "few", "more", "most", "other", "some",
        "such", "only", "own", "same", "than", "too", "very", "just",
        "also", "then", "now", "here", "there", "when", "where", "why",
        "how", "what", "which", "who", "whom", "isn", "aren", "wasn",
        "weren", "hasn", "haven", "hadn", "doesn", "don", "didn",
        "won", "wouldn", "can", "couldn", "shouldn", "mayn", "mightn",
        "needn", "dare", "ought"
    )

    private val STOP_WORDS = CJK_STOP_WORDS + ENGLISH_STOP_WORDS

    fun extractKeywords(input: String): List<String> {
        val keywords = mutableSetOf<String>()
        val currentWord = StringBuilder()
        val cjkAccumulator = StringBuilder()

        fun flushWord() {
            if (currentWord.isNotEmpty()) {
                val word = currentWord.toString().lowercase()
                if (word.length >= 2 && word !in STOP_WORDS) {
                    keywords.add(word)
                }
                currentWord.clear()
            }
        }

        fun flushCjk() {
            if (cjkAccumulator.isNotEmpty()) {
                pushCjkKeywords(cjkAccumulator.toString(), keywords)
                cjkAccumulator.clear()
            }
        }

        for (ch in input) {
            when {
                ch.isLetterOrDigit() && !isCjk(ch) -> {
                    flushCjk()
                    currentWord.append(ch)
                }
                isCjk(ch) -> {
                    flushWord()
                    cjkAccumulator.append(ch)
                }
                else -> {
                    flushWord()
                    flushCjk()
                }
            }
        }
        flushWord()
        flushCjk()

        return keywords.sorted()
    }

    private fun pushCjkKeywords(text: String, keywords: MutableSet<String>) {
        for (i in 0 until text.length - 1) {
            val bigram = text.substring(i, i + 2)
            if (bigram !in STOP_WORDS) {
                keywords.add(bigram)
            }
        }

        for (i in 0 until text.length - 2) {
            val trigram = text.substring(i, i + 3)
            if (trigram !in STOP_WORDS) {
                keywords.add(trigram)
            }
        }

        if (text.length == 1 && text !in STOP_WORDS) {
            keywords.add(text)
        }
    }

    fun formatMemoriesForPrompt(memories: List<String>): String {
        if (memories.isEmpty()) return ""
        val lines = memories.joinToString("\n") { "- $it" }
        return "以下是关于用户的长期记忆，请在回答时参考：\n$lines"
    }
}
