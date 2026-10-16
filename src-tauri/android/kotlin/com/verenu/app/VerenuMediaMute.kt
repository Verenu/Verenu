package com.verenu.app

internal interface VerenuMediaMutePersistence {
    fun readOwnedVolume(): Int?
    fun saveOwnedVolume(volume: Int): Boolean
    fun clearOwnedVolume(): Boolean
}

internal class VerenuMediaMute(
    private val isMuted: () -> Boolean,
    private val readVolume: () -> Int,
    private val writeVolume: (Int) -> Unit,
    private val persistence: VerenuMediaMutePersistence,
) {
    private var previousVolume: Int? = null
    private var userOverrideUntilUnmuted = false

    @Synchronized
    fun update(muted: Boolean) {
        if (muted) {
            if (userOverrideUntilUnmuted) return
            if (previousVolume != null) {
                if (readVolume() != 0) {
                    clearOwnedVolumeOrThrow()
                    previousVolume = null
                    userOverrideUntilUnmuted = true
                }
                return
            }

            val savedVolume = persistence.readOwnedVolume()
            if (savedVolume != null) {
                if (readVolume() != 0) {
                    clearOwnedVolumeOrThrow()
                    userOverrideUntilUnmuted = true
                    return
                }
                previousVolume = savedVolume
                return
            }

            if (isMuted()) return
            val volume = readVolume()
            if (volume == 0) return
            if (!persistence.saveOwnedVolume(volume)) {
                throw IllegalStateException("Could not save the media volume before muting")
            }
            previousVolume = volume
            try {
                writeVolume(0)
            } catch (error: RuntimeException) {
                previousVolume = null
                clearOwnedVolumeOrThrow()
                throw error
            }
            return
        }

        val volume = previousVolume ?: persistence.readOwnedVolume()
        if (volume == null) {
            userOverrideUntilUnmuted = false
            return
        }
        if (readVolume() == 0) writeVolume(volume)
        clearOwnedVolumeOrThrow()
        previousVolume = null
        userOverrideUntilUnmuted = false
    }

    private fun clearOwnedVolumeOrThrow() {
        if (!persistence.clearOwnedVolume()) {
            throw IllegalStateException("Could not clear the saved media volume")
        }
    }
}
