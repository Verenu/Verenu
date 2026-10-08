package com.verenu.app

import android.content.Context
import android.media.AudioManager
import android.util.Log

/** Temporarily quiet media without changing ring, call, or alarm audio. */
class VerenuAudioMute(context: Context) {
    private val audio = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
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
        try {
            mediaMute.update(muted)
        } catch (error: RuntimeException) {
            Log.w("VerenuAudioMute", "Could not update media volume", error)
        }
    }

    private companion object {
        const val PREFERENCES = "verenu_media_mute"
        const val OWNED_VOLUME = "owned_media_volume"
    }
}
