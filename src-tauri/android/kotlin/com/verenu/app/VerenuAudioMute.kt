package com.verenu.app

import android.content.Context
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.util.Log

/** Temporarily quiet media without changing ring, call, or alarm audio. */
class VerenuAudioMute(context: Context) {
    private val audio = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    private val handler = Handler(Looper.getMainLooper())
    private val lifecycle = VerenuMediaMuteLifecycle(::applyMute)
    private val observeVolume = object : Runnable {
        override fun run() {
            synchronized(this@VerenuAudioMute) {
                if (lifecycle.poll()) handler.postDelayed(this, lifecycle.pollDelayMs)
            }
        }
    }
    private val mediaMute = VerenuMediaMute(
        isMuted = { audio.isStreamMute(AudioManager.STREAM_MUSIC) },
        readVolume = { audio.getStreamVolume(AudioManager.STREAM_MUSIC) },
        writeVolume = { audio.setStreamVolume(AudioManager.STREAM_MUSIC, it, 0) },
        persistence = object : VerenuMediaMutePersistence {
            override fun readOwnedVolume(): Int? {
                if (!preferences.contains(OWNED_VOLUME)) return null
                val saved = preferences.getInt(OWNED_VOLUME, 0)
                if (saved > 0) return saved
                preferences.edit().remove(OWNED_VOLUME).commit()
                return null
            }

            override fun saveOwnedVolume(volume: Int): Boolean =
                preferences.edit().putInt(OWNED_VOLUME, volume).commit()

            override fun clearOwnedVolume(): Boolean =
                preferences.edit().remove(OWNED_VOLUME).commit()
        },
    )

    @Synchronized
    fun update(muted: Boolean) {
        handler.removeCallbacks(observeVolume)
        if (lifecycle.update(muted)) handler.postDelayed(observeVolume, lifecycle.pollDelayMs)
    }

    private fun applyMute(muted: Boolean): Boolean =
        try {
            mediaMute.update(muted)
            true
        } catch (error: RuntimeException) {
            Log.w("VerenuAudioMute", "Could not update media volume", error)
            false
        }

    private companion object {
        const val PREFERENCES = "verenu_media_mute"
        const val OWNED_VOLUME = "owned_media_volume"
    }
}
