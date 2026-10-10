package com.verenu.app

import org.json.JSONObject

// Final allowlist boundary shared by production analytics and JVM regressions.
internal fun safeAnalyticsSettings(json: JSONObject?): Map<String, Any> {
  if (json == null) return emptyMap()
  val booleans = listOf(
    "cleanup_enabled", "voice_commands_enabled", "dual_transcription_enabled", "noise_reduction", "mute_audio",
    "exclusive_mic", "pause_media", "sound_effects",
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
    result["feature_breadth"] = json.optInt("feature_breadth", 0).coerceIn(0, 10)
  }
  booleans.forEach { if (json.opt(it) is Boolean) result[it] = json.optBoolean(it) }
  categories.forEach {
    val value = json.optString(it, "")
    val normalized = when (it) {
      "transcription_provider", "cleanup_provider" -> when (value) {
        "groq", "openai", "google", "assemblyai", "openrouter", "xai", "local" -> value
        else -> "unknown"
      }
      "cleanup_intensity" -> when (value) {
        "none", "rules", "light", "medium", "high" -> value
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
