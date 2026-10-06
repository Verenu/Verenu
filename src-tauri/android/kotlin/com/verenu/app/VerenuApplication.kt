package com.verenu.app

import android.app.Application
import android.util.Log

class VerenuApplication : Application() {
  override fun onCreate() {
    super.onCreate()

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
