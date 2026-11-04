package com.verenu.app

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import java.util.Locale
import java.security.SecureRandom
import java.util.UUID

class VerenuKeystore(appContext: Context) {
    companion object {
        const val TAG = "VerenuKeystore"
        const val PREFS_FILE = "verenu_secrets"
        const val MAX_CREDENTIAL_BYTES = 16 * 1024
        private val legalIdentityLock = Any()
        val consentLock = Any()

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

    fun has(provider: String): Boolean = read(provider).isNotEmpty()

    fun legalIdentity(): Pair<String, String> = synchronized(legalIdentityLock) {
        val oldId = prefs.getString("legal_install_id", null)
        val oldSecret = prefs.getString("legal_install_secret", null)
        if (oldId != null && oldSecret != null) return@synchronized Pair(oldId, oldSecret)
        check(oldId == null && oldSecret == null) { "Incomplete installation identity" }
        val id = UUID.randomUUID().toString()
        val bytes = ByteArray(32).also { SecureRandom().nextBytes(it) }
        val secret = bytes.joinToString("") { "%02x".format(it.toInt() and 0xff) }
        check(prefs.edit().putString("legal_install_id", id).putString("legal_install_secret", secret).commit())
        Pair(id, secret)
    }

    fun deleteProviderCredentials() {
        val editor = prefs.edit()
        for (key in prefs.all.keys) {
            if (key.startsWith("api_key_")) editor.remove(key)
        }
        check(editor.commit()) { "Could not remove credentials" }
    }

    fun deleteLegalIdentity() = synchronized(legalIdentityLock) {
        check(prefs.edit().remove("legal_install_id").remove("legal_install_secret").commit()) {
            "Could not remove installation identity"
        }
    }

    fun readAll(): Map<String, String> =
        KNOWN_PROVIDERS.associateWith(::read).filterValues { it.isNotEmpty() }

    fun delete(provider: String) {
        check(prefs.edit().remove(keyFor(provider)).commit()) {
            "Could not remove credential for $provider"
        }
    }

    fun pushAll(bridge: VerenuBridge) {
        for (provider in KNOWN_PROVIDERS) {
            val key = read(provider)
            bridge.pushCredential(provider, key)
        }
    }
}
