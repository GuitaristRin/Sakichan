package com.sakichan.se.data.repository

import android.content.Context
import com.sakichan.se.core.model.PendingCompletion
import kotlinx.serialization.json.Json

/**
 * 持久化 [PendingCompletion]:前台服务写入,ViewModel 读取后清除。
 *
 * 用 SharedPreferences(非 DataStore):单条 write-once-read-once 记录,
 * SharedPreferences 的同步 API 更简单,无协程开销。
 * 进程被杀后数据仍在磁盘,下次启动可恢复。
 */
class PendingCompletionStore(
    context: Context,
    private val json: Json,
) {
    private val prefs = context.getSharedPreferences(PREF_FILE, Context.MODE_PRIVATE)

    fun save(pc: PendingCompletion) {
        prefs.edit().putString(KEY, json.encodeToString(PendingCompletion.serializer(), pc)).apply()
    }

    fun load(): PendingCompletion? {
        val raw = prefs.getString(KEY, null) ?: return null
        return runCatching { json.decodeFromString(PendingCompletion.serializer(), raw) }.getOrNull()
    }

    fun clear() {
        prefs.edit().remove(KEY).apply()
    }

    private companion object {
        const val PREF_FILE = "sakichan_pending"
        const val KEY = "pending_completion"
    }
}
