package com.verenu.app

import android.app.Activity
import android.app.ActivityOptions
import android.app.AlarmManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
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

@TauriPlugin
class VerenuSecurityPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun restartForDeletion(invoke: Invoke) {
    try {
      val intent = activity.packageManager.getLaunchIntentForPackage(activity.packageName)
        ?: error("No launch activity")
      intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
      val options = if (android.os.Build.VERSION.SDK_INT >= 35) {
        ActivityOptions.makeBasic()
          .setPendingIntentCreatorBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
          .toBundle()
      } else null
      val pending = PendingIntent.getActivity(activity, 0, intent, PendingIntent.FLAG_CANCEL_CURRENT or PendingIntent.FLAG_IMMUTABLE, options)
      val alarms = activity.getSystemService(Context.ALARM_SERVICE) as AlarmManager
      alarms.set(AlarmManager.ELAPSED_REALTIME, SystemClock.elapsedRealtime() + 1500, pending)
      invoke.resolve()
      Handler(Looper.getMainLooper()).postDelayed({
        activity.finishAndRemoveTask()
        android.os.Process.killProcess(android.os.Process.myPid())
      }, 300)
    } catch (error: Exception) {
      invoke.reject("Could not restart for deletion")
    }
  }

  @Command
  fun revokeAnalytics(invoke: Invoke) {
    synchronized(VerenuKeystore.consentLock) { VerenuAnalytics.setEnabled(false) }
    invoke.resolve()
  }
  companion object {
    const val TAG = "VerenuSecurity"
  }

  @Command
  fun legalIdentity(invoke: Invoke) {
    try {
      val (id, secret) = VerenuKeystore(activity).legalIdentity()
      invoke.resolveObject(mapOf("id" to id, "secret" to secret))
    } catch (e: Exception) {
      Log.e(TAG, "secure installation identity unavailable", e)
      invoke.reject("Secure installation identity is unavailable")
    }
  }

  @Command
  fun deleteProviderCredentials(invoke: Invoke) {
    try { VerenuKeystore(activity).deleteProviderCredentials(); invoke.resolve() }
    catch (e: Exception) { invoke.reject("Secure credential deletion failed") }
  }

  @Command
  fun deleteLegalIdentity(invoke: Invoke) {
    try { VerenuKeystore(activity).deleteLegalIdentity(); invoke.resolve() }
    catch (e: Exception) { invoke.reject("Secure identity deletion failed") }
  }

  @Command
  fun saveCredential(invoke: Invoke) {
    val args = invoke.parseArgs(CredentialWriteArgs::class.java)
    val provider = args.provider.trim().lowercase(Locale.ROOT)
    if (!VerenuKeystore.KNOWN_PROVIDERS.contains(provider)) {
      invoke.reject("Unknown provider")
      return
    }
    Thread {
      try {
        writeCredentialWithConsent(args.key, { VerenuBridge(activity).hasNetworkConsent() }) {
          VerenuKeystore(activity).save(provider, args.key)
        }
        invoke.resolve()
      } catch (e: Exception) {
        if (e.message == "consent_required") { invoke.reject("consent_required"); return@Thread }
        Log.e(TAG, "secure credential write failed provider=$provider", e)
        invoke.reject("Secure credential storage is unavailable")
      }
    }.start()
  }

  @Command
  fun getCredentialStatus(invoke: Invoke) {
    try {
      val store = VerenuKeystore(activity)
      val status = mutableMapOf<String, Boolean>()
      for (provider in VerenuKeystore.KNOWN_PROVIDERS) {
        status[provider] = store.has(provider)
      }
      invoke.resolveObject(status)
    } catch (e: Exception) {
      Log.e(TAG, "secure credential status unavailable", e)
      invoke.reject("Secure credential storage is unavailable")
    }
  }
}
