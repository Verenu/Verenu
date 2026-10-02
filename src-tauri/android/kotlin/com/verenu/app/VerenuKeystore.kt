package com.verenu.app

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import java.util.Locale

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
        const val MAX_CREDENTIAL_BYTES = 16 * 1024

        /** Provider ids must match `src-tauri/src/data/store` keys. */
        val KNOWN_PROVIDERS = listOf("groq", "openai", "google", "assemblyai")

        fun keyFor(provider: String): String =
            "api_key_" + provider.trim().lowercase(Locale.ROOT)
    }

    private val prefs: SharedPreferences = try {
        val masterKey = MasterKey.Builder(appContext)
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
        val normalizedProvider = provider.trim().lowercase(Locale.ROOT)
        check(KNOWN_PROVIDERS.contains(normalizedProvider)) { "Unknown provider" }
        check(key.toByteArray(Charsets.UTF_8).size <= MAX_CREDENTIAL_BYTES) {
            "Credential is too large"
        }
        val name = keyFor(normalizedProvider)
        val persisted = if (key.isEmpty()) {
            prefs.edit().remove(name).commit()
        } else {
            prefs.edit().putString(name, key).commit()
        }
        check(persisted) { "Could not persist credential for $provider" }
    }

    fun read(provider: String): String = prefs.getString(keyFor(provider), "") ?: ""

    /** Status-only lookup for the Settings UI; the secret never crosses IPC. */
    fun has(provider: String): Boolean = read(provider).isNotEmpty()

    fun readAll(): Map<String, String> =
        KNOWN_PROVIDERS.associateWith(::read).filterValues { it.isNotEmpty() }

    fun delete(provider: String) {
        check(prefs.edit().remove(keyFor(provider)).commit()) {
            "Could not remove credential for $provider"
        }
    }

    /**
     * Push every stored credential into Rust's memory cache. Called at
     * service connect (unlock) and after any rotation, so a Rust restart
     * behind a live service rehydrates without asking the user again.
     */
    fun pushAll(bridge: VerenuBridge) {
        for (provider in KNOWN_PROVIDERS) {
            val key = read(provider)
            // Send empty values too. This is the revocation path: without it,
            // deleting a key from EncryptedSharedPreferences left the old
            // value alive in Rust's memory cache until the next process boot.
            bridge.pushCredential(provider, key)
        }
    }
}
