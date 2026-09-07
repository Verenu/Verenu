package com.verenu.app

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/**
 * Durable credential storage: `EncryptedSharedPreferences` backed by the
 * Android Keystore. This is the ONLY place API keys rest on Android — the
 * Rust backend holds them in memory alone (see `src-tauri/src/android/`).
 *
 * Sync protocol with Rust (all over the authed loopback bridge):
 * - Boot/unlock: [pushAll] reads every known provider and POSTs it to
 *   `/v1/credential`, populating Rust's memory cache.
 * - Save/delete from the main app: Rust stages a rotation; the service sees
 *   `keystorePending` in the state poll, fetches it once via
 *   [VerenuBridge.takeKeystorePending], and persists it here.
 * - Revocation: clearing here + [pushAll] with empty values drops Rust's copy.
 *
 * Fail-closed: if the Keystore is unusable (broken TEE on some OEM builds),
 * reads/writes throw and the caller surfaces onboarding recovery instead of
 * silently downgrading to plaintext.
 *
 * Requires `androidx.security:security-crypto` (added to the app module by
 * `scripts/android-sync.mjs`).
 */
class VerenuKeystore(appContext: Context) {
    companion object {
        const val TAG = "VerenuKeystore"
        const val PREFS_FILE = "verenu_secrets"

        /** Provider ids must match `src-tauri/src/data/store` keys. */
        val KNOWN_PROVIDERS = listOf("groq", "openai", "google", "assemblyai")

        fun keyFor(provider: String): String = "api_key_" + provider.trim().lowercase()
    }

    private val prefs: SharedPreferences = try {
        val masterKey = MasterKey.Builder(appContext, MasterKey.AES256_GCM_SPEC)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()
        EncryptedSharedPreferences.create(
            appContext,
            PREFS_FILE,
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    } catch (e: Exception) {
        Log.e(TAG, "Android Keystore unavailable; refusing plaintext fallback", e)
        throw IllegalStateException("Device credential storage is unavailable", e)
    }

    fun save(provider: String, key: String) {
        val name = keyFor(provider)
        if (key.isEmpty()) {
            prefs.edit().remove(name).apply()
        } else {
            prefs.edit().putString(name, key).apply()
        }
    }

    fun read(provider: String): String = prefs.getString(keyFor(provider), "") ?: ""

    fun readAll(): Map<String, String> =
        KNOWN_PROVIDERS.associateWith(::read).filterValues { it.isNotEmpty() }

    fun delete(provider: String) {
        prefs.edit().remove(keyFor(provider)).apply()
    }

    /**
     * Push every stored credential into Rust's memory cache. Called at
     * service connect (unlock) and after any rotation, so a Rust restart
     * behind a live service rehydrates without asking the user again.
     */
    fun pushAll(bridge: VerenuBridge) {
        for (provider in KNOWN_PROVIDERS) {
            val key = read(provider)
            if (key.isNotEmpty()) {
                bridge.pushCredential(provider, key)
            }
        }
    }
}
