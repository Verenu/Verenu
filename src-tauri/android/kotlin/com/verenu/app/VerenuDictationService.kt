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
import android.util.Log
import androidx.core.app.NotificationCompat

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
                VerenuBridge(this).stopRecording()
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_CANCEL -> {
                VerenuBridge(this).cancelRecording()
                stopSelf()
                return START_NOT_STICKY
            }
            else -> {
                startAsMicrophoneService()
                return START_STICKY
            }
        }
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
            stopSelf()
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
            stopSelf()
        }
    }
}
