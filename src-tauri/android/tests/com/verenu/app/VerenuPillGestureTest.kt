package com.verenu.app

import com.verenu.app.VerenuPillGesture.Event
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VerenuPillGestureTest {
    @Test fun cancelledHoldEndsAndAllowsAccessibilityClick() {
        val g = VerenuPillGesture()
        g.down()
        g.longPress(idle = true, recording = false)
        assertEquals(listOf(Event.HOLD_END), g.up(cancelled = true))
        assertFalse(g.consumeClickSuppression())
    }

    @Test fun cancelledDragEndsAndAllowsAccessibilityClick() {
        val g = VerenuPillGesture()
        g.down()
        g.movedPastSlop(idle = true)
        assertEquals(listOf(Event.DRAG_END), g.up(cancelled = true))
        assertFalse(g.consumeClickSuppression())
    }

    @Test fun accessibilityClickAfterHoldReleaseIsNotSuppressed() {
        val g = VerenuPillGesture()
        g.down()
        g.longPress(idle = true, recording = false)
        g.up()
        assertTrue(g.consumeClickSuppression())
        assertFalse(g.consumeClickSuppression())
    }

    @Test fun accessibilityClickAfterDragReleaseIsNotSuppressed() {
        val g = VerenuPillGesture()
        g.down()
        g.movedPastSlop(idle = true)
        g.up()
        assertTrue(g.consumeClickSuppression())
        assertFalse(g.consumeClickSuppression())
    }

    @Test fun stationaryHoldOnIdleDictatesUntilRelease() {
        val g = VerenuPillGesture()
        g.down()
        assertEquals(Event.HOLD_START, g.longPress(idle = true, recording = false))
        assertTrue(g.suppressClick)
        assertEquals(listOf(Event.HOLD_END), g.up())
    }

    @Test fun movingBeforeTheHoldDragsInstead() {
        val g = VerenuPillGesture()
        g.down()
        assertEquals(Event.DRAG_START, g.movedPastSlop(idle = true))
        assertEquals(Event.NONE, g.longPress(idle = true, recording = false))
        assertEquals(listOf(Event.DRAG_END), g.up())
    }

    @Test fun movingAfterTheHoldKeepsDictating() {
        val g = VerenuPillGesture()
        g.down()
        g.longPress(idle = true, recording = false)
        assertEquals(Event.NONE, g.movedPastSlop(idle = true))
        assertEquals(listOf(Event.HOLD_END), g.up())
    }

    @Test fun holdingTheRecordingPillDragsIt() {
        val g = VerenuPillGesture()
        g.down()
        assertEquals(Event.DRAG_START, g.longPress(idle = false, recording = true))
        assertEquals(listOf(Event.DRAG_END), g.up())
    }

    @Test fun quickMoveOnRecordingPillDoesNotDrag() {
        val g = VerenuPillGesture()
        g.down()
        assertEquals(Event.NONE, g.movedPastSlop(idle = false))
        assertTrue(g.up().isEmpty())
    }

    @Test fun plainTapIsNotSuppressed() {
        val g = VerenuPillGesture()
        g.down()
        assertTrue(g.up().isEmpty())
        assertFalse(g.suppressClick)
    }

    @Test fun otherStatesIgnoreHolds() {
        val g = VerenuPillGesture()
        g.down()
        assertEquals(Event.NONE, g.longPress(idle = false, recording = false))
        assertTrue(g.up().isEmpty())
    }

    @Test fun cancelEndsAHoldLikeARelease() {
        val g = VerenuPillGesture()
        g.down()
        g.longPress(idle = true, recording = false)
        assertEquals(listOf(Event.HOLD_END), g.up())
        assertTrue(g.up().isEmpty())
    }

    @Test fun longPressAfterReleaseIsIgnored() {
        val g = VerenuPillGesture()
        g.down()
        g.up()
        assertEquals(Event.NONE, g.longPress(idle = true, recording = false))
    }
}

class VerenuHoldReleaseTest {
    @Test fun releaseWhileRecordingStopsImmediately() {
        val r = VerenuHoldRelease()
        assertTrue(r.holdStart(idle = true))
        assertTrue(r.beginStart())
        assertFalse(r.startFinished(recording = true))
        assertTrue(r.holdEnd(recording = true))
    }

    @Test fun releaseBeforeStartIsAppliedWhenRecordingComes() {
        val r = VerenuHoldRelease()
        r.holdStart(idle = true)
        r.beginStart()
        assertFalse(r.holdEnd(recording = false))
        assertTrue(r.startFinished(recording = true))
    }

    @Test fun queuedReleaseSurvivesASecondStartRequest() {
        val r = VerenuHoldRelease()
        r.holdStart(idle = true)
        r.beginStart()
        r.holdEnd(recording = false)
        assertFalse("tap during startup must not start again", r.beginStart())
        assertTrue(r.startFinished(recording = true))
    }

    @Test fun failedStartDropsTheQueuedRelease() {
        val r = VerenuHoldRelease()
        r.holdStart(idle = true)
        r.beginStart()
        r.holdEnd(recording = false)
        assertFalse(r.startFinished(recording = false))
        assertTrue(r.beginStart())
        assertFalse("a later tap-started recording is not stopped", r.startFinished(recording = true))
    }

    @Test fun holdOnNonIdlePillDoesNotStartOrStop() {
        val r = VerenuHoldRelease()
        assertFalse(r.holdStart(idle = false))
        assertFalse(r.holdEnd(recording = true))
    }

    @Test fun holdDuringPendingStartDoesNotOwnIt() {
        val r = VerenuHoldRelease()
        r.beginStart()
        assertFalse(r.holdStart(idle = true))
        assertFalse(r.holdEnd(recording = false))
        assertFalse(r.startFinished(recording = true))
    }

    @Test fun secondHoldDuringPendingStartKeepsQueuedRelease() {
        val r = VerenuHoldRelease()
        r.holdStart(idle = true)
        r.beginStart()
        r.holdEnd(recording = false)
        assertFalse(r.holdStart(idle = true))
        assertFalse(r.holdEnd(recording = false))
        assertTrue(r.startFinished(recording = true))
    }
}
