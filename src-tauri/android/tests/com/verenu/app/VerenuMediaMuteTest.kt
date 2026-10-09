package com.verenu.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class VerenuMediaMuteTest {
    private class Store(var ownedVolume: Int? = null) : VerenuMediaMutePersistence {
        var failSave = false
        var failClear = false

        override fun readOwnedVolume(): Int? = ownedVolume

        override fun saveOwnedVolume(volume: Int): Boolean {
            if (failSave) return false
            ownedVolume = volume
            return true
        }

        override fun clearOwnedVolume(): Boolean {
            if (failClear) return false
            ownedVolume = null
            return true
        }
    }

    private class Media(
        var volume: Int = 7,
        var muted: Boolean = false,
        val store: Store = Store(),
    ) {
        val writes = mutableListOf<Int>()
        var failRestore = false
        val control = VerenuMediaMute(
            isMuted = { muted },
            readVolume = { volume },
            writeVolume = {
                if (it > 0 && failRestore) throw IllegalStateException("restore failed")
                volume = it
                writes.add(it)
            },
            persistence = store,
        )
    }

    @Test fun restoresOriginalVolumeAfterRepeatedPolls() {
        val media = Media()
        media.control.update(true)
        media.control.update(true)
        media.control.update(false)
        media.control.update(false)
        assertEquals(7, media.volume)
        assertEquals(listOf(0, 7), media.writes)
        assertNull(media.store.ownedVolume)
    }

    @Test fun alreadySilentOrMutedMediaStaysUntouched() {
        for (media in listOf(Media(0), Media(7, true))) {
            media.control.update(true)
            media.control.update(false)
            assertTrue(media.writes.isEmpty())
            assertNull(media.store.ownedVolume)
        }
    }

    @Test fun manualVolumeChangeOverridesRestore() {
        val media = Media()
        media.control.update(true)
        media.volume = 4
        media.control.update(false)
        assertEquals(4, media.volume)
        assertEquals(listOf(0), media.writes)
        assertNull(media.store.ownedVolume)
    }

    @Test fun observedUserOverrideIsKeptEvenIfTheUserThenChoosesZero() {
        val media = Media()
        media.control.update(true)
        media.volume = 4
        media.control.update(true)
        media.volume = 0
        media.control.update(true)
        media.control.update(false)
        assertEquals(0, media.volume)
        assertEquals(listOf(0), media.writes)
        assertNull(media.store.ownedVolume)
    }

    @Test fun disabledPreferenceNeverTakesOwnership() {
        val media = Media()
        repeat(3) { media.control.update(false) }
        assertEquals(7, media.volume)
        assertTrue(media.writes.isEmpty())
        assertNull(media.store.ownedVolume)
    }

    @Test fun repeatedLifecycleReleaseRestoresOnlyOwnedVolume() {
        for (userVolume in listOf<Int?>(null, 4)) {
            val media = Media()
            media.control.update(true)
            if (userVolume != null) media.volume = userVolume
            repeat(3) { media.control.update(false) }
            assertEquals(userVolume ?: 7, media.volume)
            assertEquals(if (userVolume == null) listOf(0, 7) else listOf(0), media.writes)
            assertNull(media.store.ownedVolume)
        }
    }

    @Test fun aLaterDictationCapturesTheNewVolume() {
        val media = Media()
        media.control.update(true)
        media.control.update(false)
        media.volume = 3
        media.control.update(true)
        media.control.update(false)
        assertEquals(listOf(0, 7, 0, 3), media.writes)
    }

    @Test fun processRestartRestoresOnlyTheVolumeThatVerenuOwned() {
        val store = Store()
        val beforeRestart = Media(store = store)
        beforeRestart.control.update(true)
        assertEquals(0, beforeRestart.volume)
        assertEquals(7, store.ownedVolume)

        val afterRestart = Media(volume = 0, store = store)
        afterRestart.control.update(false)
        assertEquals(7, afterRestart.volume)
        assertEquals(listOf(7), afterRestart.writes)
        assertNull(store.ownedVolume)
    }

    @Test fun processRestartRespectsAUserVolumeChangeAndClearsStaleOwnership() {
        val store = Store()
        val beforeRestart = Media(store = store)
        beforeRestart.control.update(true)
        val afterRestart = Media(volume = 4, store = store)
        afterRestart.control.update(false)
        assertEquals(4, afterRestart.volume)
        assertTrue(afterRestart.writes.isEmpty())
        assertNull(store.ownedVolume)
    }

    @Test fun restartUserVolumeOverrideLastsUntilUnmutedThenSuccessorCanMute() {
        val store = Store()
        val beforeRestart = Media(store = store)
        beforeRestart.control.update(true)

        val afterRestart = Media(volume = 4, store = store)
        afterRestart.control.update(true)
        afterRestart.control.update(true)
        assertEquals(4, afterRestart.volume)
        assertTrue(afterRestart.writes.isEmpty())
        assertNull(store.ownedVolume)

        afterRestart.control.update(false)
        afterRestart.control.update(true)
        assertEquals(0, afterRestart.volume)
        assertEquals(4, store.ownedVolume)
        afterRestart.control.update(false)
        assertEquals(4, afterRestart.volume)
        assertEquals(listOf(0, 4), afterRestart.writes)
        assertNull(store.ownedVolume)
    }

    @Test fun ownershipIsClearedOnlyAfterAStillMutedVolumeRestores() {
        val media = Media()
        media.control.update(true)
        media.failRestore = true
        try {
            media.control.update(false)
            throw AssertionError("restore should fail")
        } catch (_: IllegalStateException) {
            // The saved value remains available for the next service poll.
        }
        assertEquals(7, media.store.ownedVolume)

        media.failRestore = false
        media.control.update(false)
        assertEquals(7, media.volume)
        assertNull(media.store.ownedVolume)
    }

    @Test fun mediaRemainsAudibleIfTheRestoreValueCannotBeSaved() {
        val store = Store().apply { failSave = true }
        val media = Media(store = store)
        try {
            media.control.update(true)
            throw AssertionError("mute should not proceed without durable restore state")
        } catch (_: IllegalStateException) {
            // Do not mute if crash recovery state could not be persisted.
        }
        assertEquals(7, media.volume)
        assertTrue(media.writes.isEmpty())
    }
}
