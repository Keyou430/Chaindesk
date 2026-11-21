package dev.chaindesk.android

import android.app.Application

class ChainDeskApplication : Application() {
    lateinit var model: AppModel
        private set

    override fun onCreate() {
        super.onCreate()
        model = AppModel(this)
    }
}
