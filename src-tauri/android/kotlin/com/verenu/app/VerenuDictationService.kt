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

class VerenuDictationService : Service() {

    private val commandExecutor: ExecutorService = Executors.newSingleThreadExecutor()
    private val mainHandler = Handler(Looper.getMainLooper())

    companion object {
        const val TAG = "VerenuFgService"
        const val ACTION_ACTIVE = "com.verenu.app.action.DICTATION_ACTIVE"
        const val ACTION_STOP = "com.verenu.app.action.DICTATION_STOP"
        const val ACTION_CANCEL = "com.verenu.app.action.DICTATION_CANCEL"
        const val CHANNEL_ID = "verenu_dictation_quiet"
        const val LEGACY_CHANNEL_ID = "verenu_dictation"
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
                startAsMicrophoneService(startId)
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
                Log.w(TAG, "foreground action was not acknowledged")
            }
        }
    }

    override fun onDestroy() {
        commandExecutor.shutdown()
        mainHandler.removeCallbacksAndMessages(null)
        super.onDestroy()
    }

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT < 26) return
        val manager = getSystemService(NotificationManager::class.java) ?: return
        try {
            manager.deleteNotificationChannel(LEGACY_CHANNEL_ID)
        } catch (e: SecurityException) {
        }
        if (manager.getNotificationChannel(CHANNEL_ID) != null) return
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                getString(R.string.verenu_dictation_channel),
                NotificationManager.IMPORTANCE_MIN,
            ).apply {
                description = getString(R.string.verenu_dictation_channel_desc)
                setShowBadge(false)
                setSound(null, null)
                enableVibration(false)
                lockscreenVisibility = Notification.VISIBILITY_SECRET
            },
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
            .setSilent(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setPriority(NotificationCompat.PRIORITY_MIN)
            .setVisibility(NotificationCompat.VISIBILITY_SECRET)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
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

    private fun startAsMicrophoneService(startId: Int) {
        val notification = try {
            buildNotification()
        } catch (e: Exception) {
            Log.e(TAG, "cannot build notification", e)
            abortBackendRecording(startId)
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
            Log.e(TAG, "cannot enter foreground", e)
            abortBackendRecording(startId)
        }
    }

    private fun abortBackendRecording(startId: Int) {
        try {
            commandExecutor.execute {
                try {
                    VerenuBridge(this@VerenuDictationService).cancelRecording()
                } catch (e: Exception) {
                    Log.w(TAG, "could not unwind backend after foreground failure", e)
                }
            }
        } catch (e: Exception) {
            Log.w(TAG, "could not schedule backend cleanup after foreground failure", e)
        } finally {
            // A service launched with startForegroundService() must either
            // enter foreground immediately or stop before Android's deadline.
            stopSelfResult(startId)
        }
    }
}
