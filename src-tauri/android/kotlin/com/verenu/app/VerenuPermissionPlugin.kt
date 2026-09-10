package com.verenu.app

import android.Manifest
import android.accessibilityservice.AccessibilityServiceInfo
import android.app.Activity
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationManagerCompat
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import androidx.activity.result.ActivityResult

@InvokeArg
internal class PermissionRequestArgs {
  lateinit var permission: String
}

/** Native owner of Android's permission prompts and settings recovery pages. */
@TauriPlugin(
  permissions = [
    Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = "microphone"),
    Permission(strings = [Manifest.permission.POST_NOTIFICATIONS], alias = "notifications"),
  ]
)
class VerenuPermissionPlugin(private val activity: Activity) : Plugin(activity) {
  private val askedPrefs by lazy {
    activity.getSharedPreferences("verenu_permission_requests", Context.MODE_PRIVATE)
  }

  @Command
  fun snapshot(invoke: Invoke) {
    invoke.resolveObject(permissionSnapshot())
  }

  @Command
  fun request(invoke: Invoke) {
    val args = invoke.parseArgs(PermissionRequestArgs::class.java)
    when (args.permission) {
      "microphone" -> requestRuntimePermission(invoke, "microphone", Manifest.permission.RECORD_AUDIO)
      "notifications" -> {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
          invoke.resolveObject(permissionSnapshot())
        } else {
          requestRuntimePermission(invoke, "notifications", Manifest.permission.POST_NOTIFICATIONS)
        }
      }
      "accessibility_service", "battery_exemption" -> openSettings(invoke, args.permission)
      else -> invoke.reject("Unknown Android permission")
    }
  }

  private fun requestRuntimePermission(invoke: Invoke, alias: String, permission: String) {
    if (ActivityCompat.checkSelfPermission(activity, permission) == PackageManager.PERMISSION_GRANTED) {
      invoke.resolveObject(permissionSnapshot())
      return
    }

    if (isPermanentlyDenied(permission)) {
      openAppSettings(invoke)
      return
    }

    askedPrefs.edit().putBoolean(alias, true).apply()
    requestPermissionForAlias(alias, invoke, "runtimePermissionCallback")
  }

  @PermissionCallback
  private fun runtimePermissionCallback(invoke: Invoke) {
    invoke.resolveObject(permissionSnapshot())
  }

  private fun openSettings(invoke: Invoke, permission: String) {
    val intent = when (permission) {
      "accessibility_service" -> Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
      "battery_exemption" -> {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
          Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
            .setData(android.net.Uri.parse("package:${activity.packageName}"))
        } else {
          Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
        }
      }
      else -> return openAppSettings(invoke)
    }

    try {
      startActivityForResult(invoke, intent, "settingsActivityCallback")
    } catch (_: Exception) {
      // OEMs may not expose the direct battery confirmation activity.
      if (permission == "battery_exemption") {
        try {
          startActivityForResult(
            invoke,
            Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS),
            "settingsActivityCallback",
          )
          return
        } catch (_: Exception) {
          // Fall through to the app settings page, which exists on supported Android versions.
        }
      }
      openAppSettings(invoke)
    }
  }

  private fun openAppSettings(invoke: Invoke) {
    val intent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
      .setData(android.net.Uri.parse("package:${activity.packageName}"))
    try {
      startActivityForResult(invoke, intent, "settingsActivityCallback")
    } catch (_: Exception) {
      invoke.reject("Android settings are unavailable")
    }
  }

  @ActivityCallback
  private fun settingsActivityCallback(invoke: Invoke, _result: ActivityResult) {
    invoke.resolveObject(permissionSnapshot())
  }

  private fun isPermanentlyDenied(permission: String): Boolean {
    return askedPrefs.getBoolean(permission, false) &&
      !ActivityCompat.shouldShowRequestPermissionRationale(activity, permission)
  }

  private fun permissionSnapshot(): Map<String, String> {
    return mapOf(
      "microphone" to runtimeGrant("microphone", Manifest.permission.RECORD_AUDIO),
      "accessibility_service" to if (accessibilityEnabled()) "granted" else "denied",
      "battery_exemption" to if (batteryExempt()) "granted" else "denied",
      "notifications" to notificationGrant(),
    )
  }

  private fun runtimeGrant(alias: String, permission: String): String {
    if (ActivityCompat.checkSelfPermission(activity, permission) == PackageManager.PERMISSION_GRANTED) {
      return "granted"
    }
    if (!askedPrefs.getBoolean(alias, false)) return "not_asked"
    return if (isPermanentlyDenied(permission)) "permanently_denied" else "denied"
  }

  private fun notificationGrant(): String {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
      NotificationManagerCompat.from(activity).areNotificationsEnabled()
    ) {
      return "granted"
    }
    return runtimeGrant("notifications", Manifest.permission.POST_NOTIFICATIONS)
  }

  private fun accessibilityEnabled(): Boolean {
    val expected = ComponentName(activity, VerenuAccessibilityService::class.java)
      .flattenToString()
    val manager = activity.getSystemService(Context.ACCESSIBILITY_SERVICE)
      as? android.view.accessibility.AccessibilityManager ?: return false
    return manager.getEnabledAccessibilityServiceList(AccessibilityServiceInfo.FEEDBACK_ALL_MASK)
      .any { info ->
        val service = info.resolveInfo?.serviceInfo ?: return@any false
        ComponentName(service.packageName, service.name).flattenToString() == expected
      }
  }

  private fun batteryExempt(): Boolean {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) return true
    val manager = activity.getSystemService(Context.POWER_SERVICE) as? PowerManager ?: return false
    return manager.isIgnoringBatteryOptimizations(activity.packageName)
  }
}
