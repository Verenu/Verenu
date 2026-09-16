package com.verenu.app

import android.content.Context
import com.posthog.PersonProfiles
import com.posthog.PostHog
import com.posthog.android.PostHogAndroid
import com.posthog.android.PostHogAndroidConfig
import org.json.JSONObject
import java.util.UUID

/** The only boundary through which Verenu product telemetry may be emitted. */
object VerenuAnalytics {
  private const val SCHEMA_VERSION = 4
  private const val IDENTITY_SCHEMA_VERSION = 2
  private const val IDENTITY_PREFS = "verenu_analytics_identity"
  private const val INSTALL_ID_KEY = "analytics_install_id"
  private const val FIRST_SEEN_VERSION_KEY = "first_seen_version"
  private var initialized = false
  // Fail closed until the Rust bridge confirms the persisted user choice.
  private var enabled = false
  private var appContext: Context? = null
  private var analyticsInstallId: String? = null
  private var firstSeenVersion: String? = null
  private var analyticsSessionId = UUID.randomUUID().toString()
  private val reportedFailures = mutableSetOf<String>()
  private var settingsSnapshotSent = false

  @Synchronized
  fun initialize(context: Context, projectToken: String, host: String) {
    if (initialized) return
    appContext = context.applicationContext
    PostHogAndroid.setup(
      context.applicationContext,
      PostHogAndroidConfig(apiKey = projectToken, host = host).apply {
        captureApplicationLifecycleEvents = false
        captureDeepLinks = false
        captureScreenViews = false
        capturePushNotificationSubscriptions = false
        capturePushNotificationOpened = false
        sessionReplay = false
        preloadFeatureFlags = false
        sendFeatureFlagEvent = false
        setDefaultPersonProperties = false
        personProfiles = PersonProfiles.NEVER
        optOut = true
        errorTrackingConfig.autoCapture = false
        // Once enabled, identify() replaces the SDK's anonymous seed with
        // Verenu's durable analytics-only installation ID. This prevents a
        // second SDK-owned alias from splitting one installation's events.
        reuseAnonymousId = true
        getAnonymousId = { UUID.randomUUID() }
      },
    )
    // The SDK starts opted out. The Rust bridge calls setEnabled after it has
    // read the persisted product setting; no identity is created before then.
    PostHog.reset()
    initialized = true
  }

  @Synchronized
  fun setEnabled(value: Boolean) {
    if (!initialized || enabled == value) return
    enabled = value
    if (value) {
      activateIdentity()
    } else {
      deactivateIdentity()
    }
    reportedFailures.clear()
    settingsSnapshotSent = false
  }

  /** Loads or creates the analytics-only identity in app-private durable storage. */
  @Synchronized
  private fun activateIdentity() {
    val context = appContext ?: return
    val preferences = context.getSharedPreferences(IDENTITY_PREFS, Context.MODE_PRIVATE)
    val existing = preferences.getString(INSTALL_ID_KEY, null)
    val installId = existing?.takeIf(::isUuid) ?: UUID.randomUUID().toString()
    val existingVersion = preferences.getString(FIRST_SEEN_VERSION_KEY, null)
    val version = existingVersion?.takeIf(::isOfficialVersion) ?: BuildConfig.VERSION_NAME
    preferences.edit()
      .putString(INSTALL_ID_KEY, installId)
      .putString(FIRST_SEEN_VERSION_KEY, version)
      .commit()
    analyticsInstallId = installId
    firstSeenVersion = version
    analyticsSessionId = UUID.randomUUID().toString()
    PostHog.optIn()
    PostHog.reset()
    // identify() sets the event distinct ID without adding person properties;
    // PersonProfiles.NEVER remains enabled in the SDK configuration.
    PostHog.identify(installId, emptyMap(), emptyMap())
  }

  @Synchronized
  private fun deactivateIdentity() {
    PostHog.optOut()
    PostHog.reset()
    appContext?.getSharedPreferences(IDENTITY_PREFS, Context.MODE_PRIVATE)
      ?.edit()
      ?.clear()
      ?.commit()
    analyticsInstallId = null
    firstSeenVersion = null
    analyticsSessionId = UUID.randomUUID().toString()
  }

  fun newRunId(): String = UUID.randomUUID().toString()

  fun dictationStarted(runId: String, directInsertionAvailable: Boolean, settings: JSONObject?) {
    reportedFailures.clear()
    val safeSettings = safeSettings(settings)
    if (!settingsSnapshotSent && safeSettings.isNotEmpty()) {
      capture("settings_snapshot", safeSettings)
      settingsSnapshotSent = true
    }
    capture("dictation_started", mapOf(
      "run_id" to runId,
      "direct_insertion_available" to directInsertionAvailable,
    ))
  }

  fun recordingFinished(runId: String, durationMs: Long) = capture("recording_finished", mapOf(
    "run_id" to runId,
    "recording_duration_bucket" to durationBucket(durationMs),
  ))

  fun dictationStopped(runId: String) = capture("dictation_stopped", mapOf("run_id" to runId))
  fun dictationCancelled(runId: String) = capture("dictation_cancelled", mapOf("run_id" to runId))
  fun dictationInserted(runId: String, method: String, wordCount: Int) = capture("dictation_inserted", mapOf(
    "run_id" to runId,
    "delivery_method" to safeDeliveryMethod(method),
    "word_count" to wordCount.coerceIn(0, 10_000),
  ))

  fun insertionAttempted(runId: String) = capture("insertion_attempted", mapOf("run_id" to runId))

  fun pipelineStageStarted(runId: String, stage: String) = capture("pipeline_stage_started", mapOf(
    "run_id" to runId,
    "pipeline_stage" to safeStage(stage),
  ))

  fun pipelineFailed(runId: String, message: String, stage: String) {
    val category = failureCategory(message)
    val safeStage = safeStage(stage)
    // Suppress duplicate bridge observations for this attempt, while allowing
    // the same failure in a later attempt to remain analytically visible.
    val fingerprint = "$runId:$category:$safeStage"
    if (!reportedFailures.add(fingerprint)) return
    capture("pipeline_failed", mapOf(
      "run_id" to runId,
      // Keep Android aligned with the desktop contract. The event name is
      // shared; only the value mapping is platform-specific.
      "category" to category,
      "stage" to safeStage,
    ))
    // The raw bridge message can contain provider text, endpoint details, or
    // user-controlled data. Map it locally and send only a fixed error family.
    if (category !in setOf("vad_rejected", "permission_missing")) {
      captureSanitizedException(
        code = when (safeStage) {
          "transcribing" -> "transcription_failed"
          "cleaning" -> "cleanup_failed"
          "inserting", "pasting" -> "insertion_failed"
          else -> "capture_failed"
        },
        stage = safeStage,
        runId = runId,
      )
    }
  }

  fun retryAttempted(runId: String, kind: String) = capture("retry_attempted", mapOf(
    "run_id" to runId,
    "retry_reason" to when (kind) {
      "start", "stop", "cancel", "transcription", "insertion" -> kind
      else -> "unknown"
    },
  ))

  fun fallbackUsed(runId: String, kind: String) = capture("fallback_used", mapOf(
    "run_id" to runId,
    "fallback" to when (kind) {
      "clipboard", "manual_copy", "transcription", "cleanup" -> kind
      else -> "unknown"
    },
  ))

  fun permissionCompleted(permission: String, status: String) = capture("permission_event", mapOf(
    "permission_type" to when (permission) {
      "microphone", "notifications", "battery_exemption" -> permission
      else -> "unknown"
    },
    "permission_status" to when (status) {
      "granted", "denied", "missing", "unknown" -> status
      else -> "unknown"
    },
  ))

  /**
   * Manual Error Tracking contract. It accepts no Throwable, message, stack,
   * endpoint, file, device, or Android context value.
   */
  fun captureSanitizedException(code: String, stage: String, runId: String? = null) {
    if (!initialized || !enabled) return
    val safeCode = when (code) {
      "transcription_failed", "cleanup_failed", "insertion_failed", "capture_failed",
      "frontend_unhandled", "backend_panic", "local_model_failed" -> code
      else -> "unknown_error"
    }
    val safeStage = safeStage(stage)
    val properties = mutableMapOf<String, Any>(
      "\$exception_fingerprint" to "verenu:android:$safeCode:$safeStage",
      "\$issue_name" to "android_$safeCode",
      "\$issue_description" to "Verenu Android error: $safeCode",
      "\$exception_level" to "error",
      "\$exception_list" to listOf(mapOf(
        "type" to "Verenu.Android.$safeCode",
        "value" to safeCode,
        "mechanism" to mapOf("handled" to true, "synthetic" to false),
        "stacktrace" to mapOf(
          "type" to "raw",
          "frames" to listOf(mapOf(
            "platform" to "custom",
            "lang" to "kotlin",
            "filename" to "verenu-android",
            "function" to "android_bridge",
            "lineno" to 0,
            "in_app" to true,
          )),
        ),
      )),
      "error_domain" to "android",
      "error_code" to safeCode,
      "error_stage" to safeStage,
      "error_severity" to "error",
      "handled" to true,
      "recovered" to false,
      "recovery_method" to "none",
      "error_callsite" to "android_bridge",
    )
    properties.putAll(commonProperties())
    if (runId != null) properties["run_id"] = runId
    // PostHog's supported manual API recognizes this as an Error Tracking
    // exception. The Exception contains only the fixed safe code; no raw
    // Throwable, bridge error, provider response, or user content is passed.
    PostHog.captureException(Exception(safeCode), properties)
  }

  private fun capture(event: String, properties: Map<String, Any>) {
    if (!initialized || !enabled) return
    val installId = analyticsInstallId ?: return
    PostHog.capture(
      installId,
      event,
      properties + commonProperties(),
      emptyMap(),
      emptyMap(),
      emptyMap(),
      null,
    )
  }

  private fun commonProperties(): Map<String, Any> {
    val properties = mutableMapOf<String, Any>(
      "analytics_schema_version" to SCHEMA_VERSION,
      "identity_schema_version" to IDENTITY_SCHEMA_VERSION,
      "analytics_session_id" to analyticsSessionId,
      // Processing controls: never retain the connection address, derive
      // GeoIP fields, or materialize a person profile for this identifier.
      "\$ip" to "0.0.0.0",
      "\$geoip_disable" to true,
      "\$process_person_profile" to false,
    )
    firstSeenVersion?.let { properties["first_seen_version"] = it }
    return properties
  }

  private fun isUuid(value: String): Boolean = try {
    UUID.fromString(value)
    true
  } catch (_: IllegalArgumentException) {
    false
  }

  private fun isOfficialVersion(value: String): Boolean =
    value.matches(Regex("\\d+\\.\\d+\\.\\d+"))

  private fun safeSettings(json: JSONObject?): Map<String, Any> {
    if (json == null) return emptyMap()
    val booleans = listOf(
      "cleanup_enabled", "dual_transcription_enabled", "noise_reduction", "mute_audio",
      "exclusive_mic", "pause_media", "sound_effects", "app_context_hint",
      "auto_learn_enabled", "contextual_formatting", "contextual_caps", "auto_spacing",
      "autostart_enabled", "mic_mute_button_dictation", "sync_enabled",
    )
    val categories = listOf(
      "transcription_provider", "cleanup_provider", "cleanup_intensity",
      "history_retention", "local_model_memory_policy",
    )
    val result = mutableMapOf<String, Any>()
    if (json.has("context_group_count")) {
      result["context_group_count"] = json.optInt("context_group_count", 0).coerceIn(0, 200)
    }
    if (json.has("feature_breadth")) {
      result["feature_breadth"] = json.optInt("feature_breadth", 0).coerceIn(0, 9)
    }
    booleans.forEach { if (json.opt(it) is Boolean) result[it] = json.optBoolean(it) }
    categories.forEach {
      val value = json.optString(it, "")
      val normalized = when (it) {
        "transcription_provider", "cleanup_provider" -> when (value) {
          "groq", "openai", "google", "assemblyai", "local" -> value
          else -> "unknown"
        }
        "cleanup_intensity" -> when (value) {
          "none", "light", "medium", "high" -> value
          else -> "unknown"
        }
        "history_retention" -> when (value) {
          "7 days", "30 days", "90 days", "Forever" -> value
          else -> "unknown"
        }
        "local_model_memory_policy" -> when (value) {
          "keep_loaded", "unload_after_use", "never_load" -> value
          else -> "unknown"
        }
        else -> "unknown"
      }
      result[it] = normalized
    }
    return result
  }

  private fun safeStage(stage: String): String = when (stage) {
    "recording" -> "capture"
    "transcribing" -> "transcription"
    "cleaning" -> "cleanup"
    "pasting" -> "clipboard"
    "inserting" -> "insertion"
    "capture", "vad", "preprocessing", "transcription", "dual_transcription",
    "cleanup", "formatting", "insertion", "clipboard", "local_model", "sync" -> stage
    else -> "unknown"
  }

  private fun safeDeliveryMethod(method: String): String = when (method) {
    "direct_accessibility", "clipboard_fallback" -> method
    else -> "unknown"
  }

  private fun durationBucket(durationMs: Long): String = when {
    durationMs < 1_000 -> "under_1s"
    durationMs < 5_000 -> "1_5s"
    durationMs < 15_000 -> "5_15s"
    durationMs < 30_000 -> "15_30s"
    durationMs < 60_000 -> "30_60s"
    else -> "60s_plus"
  }

  private fun failureCategory(message: String): String {
    val lower = message.lowercase()
    return when {
      "permission" in lower -> "permission_missing"
      "microphone" in lower -> "permission_missing"
      "audio" in lower -> "audio_empty"
      "vad" in lower || "speech" in lower -> "vad_rejected"
      "transcri" in lower -> "provider_unavailable"
      "cleanup" in lower -> "provider_unavailable"
      "quota" in lower || "rate limit" in lower -> "provider_unavailable"
      "auth" in lower || "api key" in lower -> "provider_unavailable"
      "timeout" in lower -> "timeout"
      "network" in lower || "connect" in lower -> "network"
      "paste" in lower || "insert" in lower -> "insertion_failed"
      "model" in lower -> "model_unavailable"
      else -> "unknown"
    }
  }
}
