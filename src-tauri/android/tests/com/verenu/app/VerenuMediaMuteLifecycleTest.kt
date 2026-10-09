package com.verenu.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VerenuMediaMuteLifecycleTest {
    @Test fun failedRestoreKeepsPollingUntilTheOwnedVolumeRestores() {
        var volume = 6
        var saved: Int? = null
        var failRestore = false
        val media = VerenuMediaMute(
            isMuted = { false },
            readVolume = { volume },
            writeVolume = {
                if (it > 0 && failRestore) throw IllegalStateException("synthetic restore failure")
                volume = it
            },
            persistence = object : VerenuMediaMutePersistence {
                override fun readOwnedVolume(): Int? = saved
                override fun saveOwnedVolume(volume: Int): Boolean { saved = volume; return true }
                override fun clearOwnedVolume(): Boolean { saved = null; return true }
            },
        )
        val lifecycle = VerenuMediaMuteLifecycle {
            try { media.update(it); true } catch (_: IllegalStateException) { false }
        }
        assertTrue(lifecycle.update(true))
        assertEquals(0, volume)
        failRestore = true
        assertTrue(lifecycle.update(false))
        assertTrue(lifecycle.poll())
        assertEquals(0, volume)
        assertEquals(6, saved)
        failRestore = false
        assertFalse(lifecycle.poll())
        assertEquals(6, volume)
        assertEquals(null, saved)
    }

    @Test fun queuedPollUsesLatestRequestAfterCancellation() {
        val calls = mutableListOf<Boolean>()
        val lifecycle = VerenuMediaMuteLifecycle { calls.add(it); true }
        assertTrue(lifecycle.update(true))
        assertFalse(lifecycle.update(false))
        assertFalse(lifecycle.poll())
        assertEquals(listOf(true, false, false), calls)
    }

    @Test fun idleSuccessDoesNotScheduleBatteryWork() {
        val lifecycle = VerenuMediaMuteLifecycle { true }
        assertFalse(lifecycle.update(false))
    }
}
