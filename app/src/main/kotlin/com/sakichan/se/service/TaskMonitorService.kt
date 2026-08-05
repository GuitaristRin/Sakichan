package com.sakichan.se.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import com.sakichan.se.MainActivity
import com.sakichan.se.R
import com.sakichan.se.core.model.PendingCompletion
import com.sakichan.se.data.network.OpencodeClient
import com.sakichan.se.data.repository.PendingCompletionStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.koin.android.ext.android.get

/**
 * 前台服务:opencode 任务执行期间保活进程 + 备份轮询。
 *
 * 作用:
 * 1. **保活**:前台服务优先级高,Android 不轻易杀,保证手机揣兜里时
 *    ViewModel 的 SSE + 轮询协程持续运行(泡咖啡场景)。
 * 2. **备份轮询**:即使 ViewModel 被回收(Activity 销毁),服务仍在轮询
 *    opencode server。检测到任务完成后:
 *    - 结果存入 [PendingCompletionStore](跨进程恢复用)
 *    - 发通知,用户点开即回 app
 *    - stopSelf
 *
 * ViewModel 在前台时也自行轮询(实时 UI 更新),服务的轮询是冗余备份,
 * 双方检测到完成都会写/清 store,无冲突:ViewModel 清(已处理),服务写(待处理)。
 */
class TaskMonitorService : Service() {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var pollJob: Job? = null

    private val opencodeClient: OpencodeClient by lazy { get() }
    private val store: PendingCompletionStore by lazy { get() }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val baseUrl = intent?.getStringExtra(EXTRA_BASE_URL) ?: run {
            stopSelf()
            return START_NOT_STICKY
        }
        val ocSessionId = intent.getStringExtra(EXTRA_OC_SESSION_ID) ?: run {
            stopSelf()
            return START_NOT_STICKY
        }
        val sakichanSessionId = intent.getStringExtra(EXTRA_SAKICHAN_SESSION_ID) ?: run {
            stopSelf()
            return START_NOT_STICKY
        }
        val instruction = intent.getStringExtra(EXTRA_INSTRUCTION) ?: ""

        startForeground(NOTIF_ID, buildNotification("Sakichan 正在监控任务", instruction))
        startPolling(baseUrl, ocSessionId, sakichanSessionId, instruction)

        return START_NOT_STICKY  // 被杀后不自动重启,由 ViewModel 重新启动
    }

    private fun startPolling(
        baseUrl: String,
        ocSessionId: String,
        sakichanSessionId: String,
        instruction: String,
    ) {
        pollJob?.cancel()
        pollJob = scope.launch {
            var waited = 0
            while (waited < POLL_TIMEOUT_SECONDS) {
                delay(POLL_INTERVAL_MS)
                waited++
                var done = false
                var failed: String? = null
                var output: String? = null

                runCatching {
                    val msgs = opencodeClient.listMessages(baseUrl, ocSessionId)
                    val assistantMsgs = msgs.filter { it.info.role == "assistant" }
                    if (assistantMsgs.isNotEmpty()) {
                        output = assistantMsgs.joinToString("\n") { m ->
                            m.parts.filter { it.type == "text" }.joinToString("") { it.text ?: "" }
                        }.trim().ifBlank { null }
                        done = assistantMsgs.any { m -> m.parts.any { it.type == "step-finish" } }
                    }
                }.onFailure { e ->
                    // 非致命:网络抖动等,下一轮重试
                    Log.w(TAG, "poll error: ${e.message}")
                }

                if (done) {
                    val result = output ?: "任务执行完毕(无文本输出)"
                    store.save(PendingCompletion(
                        baseUrl = baseUrl,
                        opencodeSessionId = ocSessionId,
                        sakichanSessionId = sakichanSessionId,
                        instruction = instruction,
                        output = result,
                    ))
                    notifyDone(instruction, result)
                    stopSelf()
                    return@launch
                }

                if (failed != null) {
                    store.save(PendingCompletion(
                        baseUrl = baseUrl,
                        opencodeSessionId = ocSessionId,
                        sakichanSessionId = sakichanSessionId,
                        instruction = instruction,
                        output = "",
                        failed = true,
                        error = failed,
                    ))
                    notifyFailed(instruction, failed!!)
                    stopSelf()
                    return@launch
                }
            }

            // 超时:存一条 failed 记录,让 ViewModel 补跑时告知用户
            store.save(PendingCompletion(
                baseUrl = baseUrl,
                opencodeSessionId = ocSessionId,
                sakichanSessionId = sakichanSessionId,
                instruction = instruction,
                output = "",
                failed = true,
                error = "任务超时(${POLL_TIMEOUT_SECONDS}s)",
            ))
            notifyFailed(instruction, "任务超时")
            stopSelf()
        }
    }

    override fun onDestroy() {
        pollJob?.cancel()
        scope.cancel()
        super.onDestroy()
    }

    // ===== 通知 =====

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "任务监控",
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                description = "opencode 任务执行状态"
            }
            getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        }
    }

    private fun buildNotification(title: String, text: String): Notification {
        val intent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val pendingIntent = PendingIntent.getActivity(
            this, 0, intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(title)
            .setContentText(text.take(80))
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .setSilent(true)
            .build()
    }

    private fun notifyDone(instruction: String, output: String) {
        val notif = buildNotification("任务完成: ${instruction.take(40)}", output.take(100))
        getSystemService(NotificationManager::class.java)
            .notify(NOTIF_DONE_ID, notif)
    }

    private fun notifyFailed(instruction: String, error: String) {
        val notif = buildNotification("任务失败: ${instruction.take(40)}", error)
        getSystemService(NotificationManager::class.java)
            .notify(NOTIF_DONE_ID, notif)
    }

    companion object {
        private const val TAG = "TaskMonitor"
        private const val CHANNEL_ID = "sakichan_task_monitor"
        private const val NOTIF_ID = 1
        private const val NOTIF_DONE_ID = 2
        private const val POLL_INTERVAL_MS = 2000L
        private const val POLL_TIMEOUT_SECONDS = 300  // 5 分钟,比 ViewModel 的 180s 更宽松

        const val EXTRA_BASE_URL = "base_url"
        const val EXTRA_OC_SESSION_ID = "oc_session_id"
        const val EXTRA_SAKICHAN_SESSION_ID = "sakichan_session_id"
        const val EXTRA_INSTRUCTION = "instruction"

        fun start(
            context: Context,
            baseUrl: String,
            ocSessionId: String,
            sakichanSessionId: String,
            instruction: String,
        ) {
            val intent = Intent(context, TaskMonitorService::class.java).apply {
                putExtra(EXTRA_BASE_URL, baseUrl)
                putExtra(EXTRA_OC_SESSION_ID, ocSessionId)
                putExtra(EXTRA_SAKICHAN_SESSION_ID, sakichanSessionId)
                putExtra(EXTRA_INSTRUCTION, instruction)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, TaskMonitorService::class.java))
        }
    }
}
