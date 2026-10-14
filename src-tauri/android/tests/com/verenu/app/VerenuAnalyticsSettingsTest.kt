package com.verenu.app

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class VerenuAnalyticsSettingsTest {
    @Test fun preservesTenFeatureBreadthAndBoundsUntrustedCounts() {
        for ((input, expected) in listOf(-10 to 0, 0 to 0, 9 to 9, 10 to 10, 500 to 10)) {
            assertEquals(expected, safeAnalyticsSettings(JSONObject().put("feature_breadth", input))["feature_breadth"])
        }
        assertEquals(0, safeAnalyticsSettings(JSONObject().put("feature_breadth", "private dictated text"))["feature_breadth"])
    }

    @Test fun preservesBasicAndExistingCleanupIntensities() {
        for (intensity in listOf("none", "rules", "light", "medium", "high")) {
            val safe = safeAnalyticsSettings(JSONObject().put("cleanup_intensity", intensity))
            assertEquals(intensity, safe["cleanup_intensity"])
        }
    }

    @Test fun preservesBothVoiceCommandBooleanValues() {
        for (enabled in listOf(false, true)) {
            val safe = safeAnalyticsSettings(JSONObject().put("voice_commands_enabled", enabled))
            assertEquals(enabled, safe["voice_commands_enabled"])
        }
    }

    @Test fun rejectsPrivateAndMistypedValuesWithoutBroadeningTheAllowlist() {
        for (value in listOf("private dictated text", "true", 1, JSONObject.NULL)) {
            val input = JSONObject().put("voice_commands_enabled", value)
                .put("cleanup_intensity", "private dictated text")
                .put("raw_text", "private dictated text")
                .put("clipboard", "private clipboard")
                .put("custom_instructions", "private instructions")
            val safe = safeAnalyticsSettings(input)
            assertFalse(safe.containsKey("voice_commands_enabled"))
            assertEquals("unknown", safe["cleanup_intensity"])
            assertFalse(safe.containsKey("raw_text"))
            assertFalse(safe.containsKey("clipboard"))
            assertFalse(safe.containsKey("custom_instructions"))
        }
    }

    @Test fun retainsExistingBoundsAndBooleanFiltering() {
        val safe = safeAnalyticsSettings(JSONObject()
            .put("cleanup_enabled", true)
            .put("dual_transcription_enabled", "true")
            .put("context_group_count", 500)
            .put("feature_breadth", -1)
            .put("transcription_provider", "local"))
        assertEquals(true, safe["cleanup_enabled"])
        assertFalse(safe.containsKey("dual_transcription_enabled"))
        assertEquals(200, safe["context_group_count"])
        assertEquals(0, safe["feature_breadth"])
        assertEquals("local", safe["transcription_provider"])
        assertTrue(safeAnalyticsSettings(null).isEmpty())
    }
}
