package com.verenu.app

import android.app.Application
import android.util.Log

class VerenuApplication : Application() {
  private val audioMute by lazy { VerenuAudioMute(this) }

  // Called by Rust's shared mute-owner lifecycle, including in-app dictation.
  // The Application survives Activity and accessibility-service teardown.
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
    // Unconfigured analytics must never stop the app from launching; builds
    // without an ingestion token (contributors, emulators) simply run without it.
    if (BuildConfig.DEBUG) {
      Log.w("VerenuApplication", "$variableName is not configured; product analytics are disabled")
    }
    return null
  }
}
