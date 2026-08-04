package com.sakichan.se.core.util

object TokenEstimator {
    private const val CHARS_PER_TOKEN = 4

    fun estimateTokens(text: String): Int {
        return (text.length + CHARS_PER_TOKEN - 1) / CHARS_PER_TOKEN
    }

    fun fitsInBudget(text: String, maxChars: Int): Boolean {
        return text.length <= maxChars
    }
}

object Base64Utils {
    fun encodeImage(bytes: ByteArray, mimeType: String): String {
        val encoded = java.util.Base64.getEncoder().encodeToString(bytes)
        return "data:$mimeType;base64,$encoded"
    }

    fun guessMimeType(path: String): String {
        return when {
            path.endsWith(".jpg") || path.endsWith(".jpeg") -> "image/jpeg"
            path.endsWith(".png") -> "image/png"
            path.endsWith(".gif") -> "image/gif"
            path.endsWith(".webp") -> "image/webp"
            path.endsWith(".bmp") -> "image/bmp"
            else -> "image/jpeg"
        }
    }
}
