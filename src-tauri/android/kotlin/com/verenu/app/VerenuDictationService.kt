package com.verenu.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationCompat
import org.json.JSONObject
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Foreground-service holder for dictation sessions.
 *
 * Actual microphone capture stays in Rust (`cpal`, same pipeline as
 * desktop). This service exists because Android attributes microphone use
 * and process priority to a foreground service: while it runs (type
 * `microphone`), the system will not kill Verenu mid-sentence, OEM battery
 * managers back off, and the user gets a persistent Stop affordance in the
 * notification shade.
 *
 * Started/stopped by [VerenuAccessibilityService] around each dictation.
 * Holds no audio resources itself, so it can never leak the microphone.
 */
class VerenuDictationService : Service() {

    private val commandExecutor: ExecutorService = Executors.newSingleThreadExecutor()
    private val mainHandler = Handler(Looper.getMainLooper())

    companion object {
        const val TAG = "VerenuFgService"
        const val ACTION_ACTIVE = "com.verenu.app.action.DICTATION_ACTIVE"
        const val ACTION_STOP = "com.verenu.app.action.DICTATION_STOP"
        const val ACTION_CANCEL = "com.verenu.app.action.DICTATION_CANCEL"
        const val CHANNEL_ID = "verenu_dictation"
        const val NOTIFICATION_ID = 4201
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        ensureChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                // The bridge is a blocking loopback HTTP client. Notification
                // actions arrive on the service main thread, so doing this
                // inline throws NetworkOnMainThreadException on modern
                // Android and leaves Rust recording. Keep the action quick and
                // finish the service only after the backend has acknowledged.
                runBackendCommand(startId) { VerenuBridge(this@VerenuDictationService).stopRecording() }
                return START_NOT_STICKY
            }
            ACTION_CANCEL -> {
                runBackendCommand(startId) { VerenuBridge(this@VerenuDictationService).cancelRecording() }
                return START_NOT_STICKY
            }
            else -> {
                startAsMicrophoneService()
                // If Android kills the process, Rust's recording session is
                // gone too. Restarting this holder with a null intent would
                // otherwise create a misleading foreground notification with
                // no live microphone session behind it.
                return START_NOT_STICKY
            }
        }
    }

    private fun runBackendCommand(startId: Int, command: () -> JSONObject?) {
        commandExecutor.execute {
            val acknowledged = try {
                command()?.optBoolean("ok") == true
            } catch (e: Exception) {
                Log.w(TAG, "foreground action failed", e)
                false
            }
            if (acknowledged) {
                mainHandler.post { stopSelfResult(startId) }
            } else {
                // Keep the foreground holder alive when the backend did not
                // acknowledge. The user can retry from the notification
                // instead of leaving Rust recording with no stop affordance.
                Log.w(TAG, "foreground action was not acknowledged")
            }
        }
    }

    override fun onDestroy() {
        commandExecutor.shutdownNow()
        mainHandler.removeCallbacksAndMessages(null)
        super.onDestroy()
    }

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT < 26) return
        val manager = getSystemService(NotificationManager::class.java) ?: return
        if (manager.getNotificationChannel(CHANNEL_ID) != null) return
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                getString(R.string.verenu_dictation_channel),
                NotificationManager.IMPORTANCE_LOW,
            ).apply { description = getString(R.string.verenu_dictation_channel_desc) },
        )
    }

    private fun actionIntent(action: String, requestCode: Int): PendingIntent {
        val intent = Intent(this, VerenuDictationService::class.java).setAction(action)
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or
            (if (Build.VERSION.SDK_INT >= 23) PendingIntent.FLAG_IMMUTABLE else 0)
        return PendingIntent.getService(this, requestCode, intent, flags)
    }

    private fun buildNotification(): Notification =
        NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(getString(R.string.verenu_recording_title))
            .setContentText(getString(R.string.verenu_recording_text))
            .setSmallIcon(android.R.drawable.presence_audio_online)
            .setOngoing(true)
            .setCategory(NotificationCompat.CATEGORY_STATUS)
            .addAction(
                android.R.drawable.ic_menu_close_clear_cancel,
                getString(R.string.verenu_action_stop),
                actionIntent(ACTION_STOP, 1),
            )
            .addAction(
                android.R.drawable.ic_delete,
                getString(R.string.verenu_action_cancel),
                actionIntent(ACTION_CANCEL, 2),
            )
            .build()

    private fun startAsMicrophoneService() {
        val notification = try {
            buildNotification()
        } catch (e: Exception) {
            Log.e(TAG, "cannot build notification", e)
            abortBackendRecording()
            return
        }
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(
                    NOTIFICATION_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
                )
            } else {
                @Suppress("DEPRECATION")
                startForeground(NOTIFICATION_ID, notification)
            }
        } catch (e: Exception) {
            // ForegroundServiceStartNotAllowedException (background start on
            // API 31+) or missing mic permission: log loudly; the Rust
            // pipeline surfaces its own error and onboarding recovery
            // explains the fix.
            Log.e(TAG, "cannot enter foreground", e)
            abortBackendRecording()
        }
    }

    /**
     * The Rust session is started immediately before this service. If Android
     * refuses to promote us to a microphone foreground service, cancel Rust's
     * session asynchronously before stopping this holder.
     */
    private fun abortBackendRecording() {
        commandExecutor.execute {
            try {
                VerenuBridge(this@VerenuDictationService).cancelRecording()
            } catch (e: Exception) {
                Log.w(TAG, "could not unwind backend after foreground failure", e)
            } finally {
                mainHandler.post { stopSelf() }
            }
        }
    }
}
