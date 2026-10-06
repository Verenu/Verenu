package com.verenu.app

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.io.File

/** Rust owns release selection and checksum verification. Android owns APK identity and installation. */
@TauriPlugin
class VerenuUpdaterPlugin(private val activity: Activity) : Plugin(activity) {
  private fun directory() = File(activity.cacheDir, "verenu-updates")

  @Command
  fun prepare(invoke: Invoke) {
    activity.runOnUiThread {
      try {
        if (!activity.packageManager.canRequestPackageInstalls()) {
          activity.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
            Uri.parse("package:${activity.packageName}")))
          invoke.reject("Allow updates from Verenu in Android settings, then return and tap Update Verenu again.")
          return@runOnUiThread
        }
        val dir = directory()
        check(dir.exists() || dir.mkdirs())
        // Keep a handed-off APK until the next explicit attempt so the system
        // installer can still read it after Verenu backgrounds or restarts.
        val apk = File(dir, "update.apk")
        check(!apk.exists() || apk.delete())
        invoke.resolveObject(mapOf("directory" to dir.absolutePath))
      } catch (_: Exception) {
        invoke.reject("Could not prepare Android updates. Check storage and try again.")
      }
    }
  }

  @Suppress("DEPRECATION")
  private fun versionCode(info: PackageInfo): Long =
    if (Build.VERSION.SDK_INT >= 28) info.longVersionCode else info.versionCode.toLong()

  @Command
  @Suppress("DEPRECATION")
  fun install(invoke: Invoke) {
    // Archive inspection can be expensive for APKs with bundled runtimes.
    Thread {
      try {
        val apk = File(directory(), "update.apk")
        val manager = activity.packageManager
        val candidate = manager.getPackageArchiveInfo(apk.absolutePath, PackageManager.GET_SIGNATURES)
          ?: error("Invalid APK")
        val installed = manager.getPackageInfo(activity.packageName, PackageManager.GET_SIGNATURES)
        val candidateSigners = candidate.signatures.orEmpty().map { it.toCharsString() }.toSet()
        val installedSigners = installed.signatures.orEmpty().map { it.toCharsString() }.toSet()
        if (!VerenuUpdatePolicy.accepts(activity.packageName, candidate.packageName,
            versionCode(installed), versionCode(candidate), installedSigners, candidateSigners)) {
          apk.delete()
          invoke.reject("This APK is not a compatible Verenu update. Its package, signing key, or version code differs.")
          return@Thread
        }
        activity.runOnUiThread {
          try {
            val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.updates", apk)
            activity.startActivity(Intent(Intent.ACTION_VIEW)
              .setDataAndType(uri, "application/vnd.android.package-archive")
              .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
            invoke.resolve()
          } catch (_: Exception) {
            apk.delete()
            invoke.reject("Android could not open its installer. Check installation permission and try again.")
          }
        }
      } catch (_: Exception) {
        File(directory(), "update.apk").delete()
        invoke.reject("Android could not validate the downloaded APK. Check for updates and try again.")
      }
    }.start()
  }
}
