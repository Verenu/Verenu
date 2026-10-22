package com.verenu.app

import android.app.Application
import android.util.Log

class VerenuApplication : Application() {
  private val audioMute by lazy { VerenuAudioMute(this) }

  fun updateMediaMute(muted: Boolean) = audioMute.update(muted)

  override fun onCreate() {
    super.onCreate()
    updateMediaMute(false)

    val projectToken = configuredValue(
      BuildConfig.POSTHOG_PROJECT_TOKEN,
      "POSTHOG_PROJECT_TOKEN",
    ) ?: return
    val host = configuredValue(BuildConfig.POSTHOG_HOST, "POSTHOG_HOST") ?: return

    VerenuAnalytics.initialize(this, projectToken, host)
  }

  private fun configuredValue(value: String?, variableName: String): String? {
    if (!value.isNullOrBlank()) return value
    if (BuildConfig.DEBUG) {
      Log.w("VerenuApplication", "$variableName is not configured; product analytics are disabled")
    }
    return null
  }
}
