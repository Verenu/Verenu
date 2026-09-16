package com.verenu.app

import android.app.Application

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
    if (BuildConfig.DEBUG) {
      error(
        "$variableName variable required by product analytics is missing or un-configured, this causes events to be silently missed. This error stops appearing once $variableName is configured",
      )
    }
    return null
  }
}
