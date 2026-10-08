package com.verenu.app

import org.junit.Assert.assertEquals
import org.junit.Test

class VerenuAccessibilityPollTest {
    @Test fun optedInHiddenServicePollsOftenEnoughToObserveShortMuteLeases() {
        assertEquals(
            VerenuAccessibilityService.POLL_MUTING_MS,
            VerenuAccessibilityService.nextPollDelayMs(
                recording = false,
                audioMuteActive = false,
                overlayAttached = false,
                syncMutingEnabled = true,
            ),
        )
        check(VerenuAccessibilityService.POLL_MUTING_MS < VerenuAccessibilityService.POLL_IDLE_MS)
    }

    @Test fun featureOffKeepsTheBatterySavingIdleInterval() {
        assertEquals(
            VerenuAccessibilityService.POLL_IDLE_MS,
            VerenuAccessibilityService.nextPollDelayMs(
                recording = false,
                audioMuteActive = false,
                overlayAttached = false,
                syncMutingEnabled = false,
            ),
        )
    }

    @Test fun activeOrVisibleWorkKeepsItsExistingPollPriority() {
        assertEquals(
            VerenuAccessibilityService.POLL_RECORDING_MS,
            VerenuAccessibilityService.nextPollDelayMs(
                recording = true,
                audioMuteActive = true,
                overlayAttached = true,
                syncMutingEnabled = true,
            ),
        )
        assertEquals(
            VerenuAccessibilityService.POLL_VISIBLE_MS,
            VerenuAccessibilityService.nextPollDelayMs(
                recording = false,
                audioMuteActive = true,
                overlayAttached = false,
                syncMutingEnabled = true,
            ),
        )
    }
}
