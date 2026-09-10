package com.verenu.app

import android.app.Activity
import android.util.Log
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.util.Locale

@InvokeArg
internal class CredentialWriteArgs {
  lateinit var provider: String
  var key: String = ""
}

/**
 * Native owner of durable API-key writes. The Rust command remains the
 * transient in-memory/cache path; this plugin makes Settings saves durable
 * even when the AccessibilityService is disabled or not yet connected.
 */
@TauriPlugin
class VerenuSecurityPlugin(private val activity: Activity) : Plugin(activity) {
  companion object {
    const val TAG = "VerenuSecurity"
  }

  @Command
  fun saveCredential(invoke: Invoke) {
    val args = invoke.parseArgs(CredentialWriteArgs::class.java)
    val provider = args.provider.trim().lowercase(Locale.ROOT)
    if (!VerenuKeystore.KNOWN_PROVIDERS.contains(provider)) {
      invoke.reject("Unknown provider")
      return
    }
    try {
      VerenuKeystore(activity).save(provider, args.key)
      invoke.resolve()
    } catch (e: Exception) {
      // Never include the key or exception payload in the user-facing error.
      Log.e(TAG, "secure credential write failed provider=$provider", e)
      invoke.reject("Secure credential storage is unavailable")
    }
  }

  @Command
  fun getCredentialStatus(invoke: Invoke) {
    try {
      val store = VerenuKeystore(activity)
      val status = mutableMapOf<String, Boolean>()
      for (provider in VerenuKeystore.KNOWN_PROVIDERS) {
        // Only booleans cross the WebView boundary. Key material remains in
        // EncryptedSharedPreferences and is never returned to JavaScript.
        status[provider] = store.has(provider)
      }
      invoke.resolveObject(status)
    } catch (e: Exception) {
      Log.e(TAG, "secure credential status unavailable", e)
      invoke.reject("Secure credential storage is unavailable")
    }
  }
}
