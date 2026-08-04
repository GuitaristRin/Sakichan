package com.sakichan.se

import android.app.Application
import com.sakichan.se.di.appModule
import org.koin.android.ext.koin.androidContext
import org.koin.core.context.startKoin

class SakichanApp : Application() {
    override fun onCreate() {
        super.onCreate()
        startKoin {
            androidContext(this@SakichanApp)
            modules(appModule)
        }
    }
}
