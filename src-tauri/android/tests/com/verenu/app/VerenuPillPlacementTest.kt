package com.verenu.app

import com.verenu.app.VerenuPillPlacement.DropContext
import com.verenu.app.VerenuPillPlacement.Snap
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class VerenuPillPlacementTest {
    private val width = 1080
    private val height = 2400

    private fun docked(position: String = "keyboard-center") = DropContext(
        screenWidth = width, screenHeight = height, docked = true,
        position = position, coveringKeyboardMic = false,
    )

    private fun onKeyboard(position: String, covering: Boolean = false) = DropContext(
        screenWidth = width, screenHeight = height, docked = false,
        position = position, coveringKeyboardMic = covering,
        keyboardLeft = 0, keyboardTop = 1500, keyboardRight = width, keyboardReach = 200,
    )

    @Test fun screenGridFollowsThirdsOfTheDisplay() {
        assertEquals("screen-top-left", VerenuPillPlacement.screenCell(10, 10, width, height))
        assertEquals("screen-top", VerenuPillPlacement.screenCell(540, 10, width, height))
        assertEquals("screen-top-right", VerenuPillPlacement.screenCell(1070, 10, width, height))
        assertEquals("screen-left", VerenuPillPlacement.screenCell(10, 1200, width, height))
        assertEquals("screen-middle", VerenuPillPlacement.screenCell(540, 1200, width, height))
        assertEquals("screen-right", VerenuPillPlacement.screenCell(1070, 1200, width, height))
        assertEquals("screen-bottom-left", VerenuPillPlacement.screenCell(10, 2390, width, height))
        assertEquals("screen-bottom", VerenuPillPlacement.screenCell(540, 2390, width, height))
        assertEquals("screen-bottom-right", VerenuPillPlacement.screenCell(1070, 2390, width, height))
    }

    @Test fun pointsOffTheDisplayStillSnapToAnEdgeCell() {
        assertEquals("screen-top-left", VerenuPillPlacement.screenCell(-80, -40, width, height))
        assertEquals("screen-bottom-right", VerenuPillPlacement.screenCell(width + 90, height + 50, width, height))
    }

    @Test fun degenerateDisplayDoesNotCrash() {
        assertEquals("screen-middle", VerenuPillPlacement.screenCell(5, 5, 0, 0))
    }

    @Test fun dockedDropUpdatesTheDockNotThePosition() {
        assertEquals(Snap("dock", "screen-right"), VerenuPillPlacement.snapFor(1060, 1200, docked()))
        assertEquals(Snap("dock", "screen-bottom-left"), VerenuPillPlacement.snapFor(5, 2300, docked("keyboard-left")))
    }

    @Test fun screenPlacedPillUpdatesThePositionEvenWhileDocked() {
        val ctx = docked("screen-top")
        assertEquals(Snap("position", "screen-left"), VerenuPillPlacement.snapFor(10, 1200, ctx))
    }

    @Test fun coveringTheMicKeyIsNotMovable() {
        assertNull(VerenuPillPlacement.snapFor(10, 1200, onKeyboard("keyboard-center", covering = true)))
    }

    @Test fun dockedPillMovesEvenIfCoverIsOn() {
        val ctx = docked().copy(coveringKeyboardMic = true)
        assertEquals(Snap("dock", "screen-left"), VerenuPillPlacement.snapFor(10, 1200, ctx))
    }

    @Test fun uncoveredKeyboardPillSnapsAcrossTheKeyboardWidth() {
        val ctx = onKeyboard("keyboard-center")
        assertEquals(Snap("position", "keyboard-left"), VerenuPillPlacement.snapFor(100, 1600, ctx))
        assertEquals(Snap("position", "keyboard-center"), VerenuPillPlacement.snapFor(540, 1600, ctx))
        assertEquals(Snap("position", "keyboard-right"), VerenuPillPlacement.snapFor(1000, 1600, ctx))
    }

    @Test fun dropJustAboveTheKeyboardStillCountsAsTheKeyboard() {
        val ctx = onKeyboard("keyboard-left")
        assertEquals(Snap("position", "keyboard-right"), VerenuPillPlacement.snapFor(1000, 1350, ctx))
    }

    @Test fun dropWellAboveTheKeyboardLeavesKeyboardMode() {
        val ctx = onKeyboard("keyboard-center")
        assertEquals(Snap("position", "screen-top-right"), VerenuPillPlacement.snapFor(1000, 100, ctx))
    }

    @Test fun narrowFloatingKeyboardUsesItsOwnBounds() {
        val ctx = onKeyboard("keyboard-center").copy(keyboardLeft = 600, keyboardRight = 1000)
        assertEquals(Snap("position", "keyboard-left"), VerenuPillPlacement.snapFor(620, 1600, ctx))
        assertEquals(Snap("position", "keyboard-right"), VerenuPillPlacement.snapFor(980, 1600, ctx))
    }

    @Test fun everySnapResultIsAValueRustAccepts() {
        val contexts = listOf(docked(), docked("screen-top"), onKeyboard("keyboard-right"), onKeyboard("screen-middle"))
        for (ctx in contexts) for (x in 0..width step 90) for (y in 0..height step 100) {
            val snap = VerenuPillPlacement.snapFor(x, y, ctx) ?: continue
            assertTrue(VerenuPillPlacement.isKnownPosition(snap.position))
            if (snap.target == "dock") assertTrue(VerenuPillPlacement.isScreenPosition(snap.position))
        }
    }

    @Test fun unknownStoredValuesFallBackToDefaults() {
        assertEquals("keyboard-center", VerenuPillPlacement.sanitizePosition("floating"))
        assertEquals("keyboard-center", VerenuPillPlacement.sanitizePosition(null))
        assertEquals("screen-bottom", VerenuPillPlacement.sanitizeDockPosition("keyboard-left"))
        assertEquals("screen-top", VerenuPillPlacement.sanitizeDockPosition("screen-top"))
        assertEquals("punch-hole", VerenuPillPlacement.sanitizeDockPosition("punch-hole"))
    }

    @Test fun onlyKeyboardPositionsFollowTheKeyboard() {
        assertTrue(VerenuPillPlacement.followsKeyboard("keyboard-right"))
        assertFalse(VerenuPillPlacement.followsKeyboard("screen-bottom"))
        assertFalse(VerenuPillPlacement.followsKeyboard("punch-hole"))
    }
}
