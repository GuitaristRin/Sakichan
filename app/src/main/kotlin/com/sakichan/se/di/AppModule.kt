package com.sakichan.se.di

import com.sakichan.se.connection.ConnectionManager
import com.sakichan.se.data.discovery.DiscoveryService
import com.sakichan.se.data.network.ChatApiClient
import com.sakichan.se.data.network.OpencodeClient
import com.sakichan.se.data.repository.AppConfigRepository
import com.sakichan.se.data.repository.ChatHistoryRepository
import com.sakichan.se.data.repository.PendingCompletionStore
import com.sakichan.se.ui.chat.ChatViewModel
import com.sakichan.se.ui.connection.ConnectionViewModel
import kotlinx.serialization.json.Json
import okhttp3.OkHttpClient
import org.koin.android.ext.koin.androidContext
import org.koin.androidx.viewmodel.dsl.viewModel
import org.koin.dsl.module
import java.util.concurrent.TimeUnit

val appModule = module {
    single {
        OkHttpClient.Builder()
            .connectTimeout(5, TimeUnit.SECONDS)   // 连接失败快速反馈,避免卡死/误判闪退
            .readTimeout(0, TimeUnit.SECONDS)  // SSE 不超时
            .pingInterval(20, TimeUnit.SECONDS)
            .build()
    }
    single {
        Json {
            ignoreUnknownKeys = true
            isLenient = true
            coerceInputValues = true
            explicitNulls = false
        }
    }
    single { ChatApiClient(get()) }
    single { OpencodeClient(get(), get()) }
    single { AppConfigRepository(androidContext()) }
    single { ChatHistoryRepository(androidContext(), get()) }
    single { PendingCompletionStore(androidContext(), get()) }
    single { DiscoveryService(androidContext()) }
    single { ConnectionManager() }
    viewModel { ConnectionViewModel(get(), get(), get()) }
    viewModel { ChatViewModel(get(), get(), get(), get(), get(), get(), androidContext()) }
}
