package com.verenu.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VerenuInsertionEditTest {
    @Test fun appendingAtTheLauncherLimitDoesNotAllowATruncatedEdit() {
        val existing = "x".repeat(61)
        val edit = VerenuInsertionEdit(existing, 61, 61, "y".repeat(44))
        assertFalse(edit.fits(100))
        assertEquals(105, edit.text.length)
    }

    @Test fun replacingASelectionCanFitInAnOtherwiseFullField() {
        val edit = VerenuInsertionEdit("x".repeat(100), 30, 70, "Hello")
        assertTrue(edit.fits(100))
        assertEquals(65, edit.text.length)
        assertEquals(35, edit.cursor)
    }

    @Test fun exactBoundaryAndUnrestrictedFieldsAllowTheWholeEdit() {
        val edit = VerenuInsertionEdit("x".repeat(90), 90, 90, "y".repeat(10))
        assertTrue(edit.fits(100))
        assertFalse(edit.fits(99))
        assertTrue(edit.fits(-1))
    }

    @Test fun emptyFieldsAndUtf16SurrogatePairsRespectAndroidLimits() {
        val edit = VerenuInsertionEdit("", 0, 0, "\uD83D\uDE00")
        assertFalse(edit.fits(0))
        assertFalse(edit.fits(1))
        assertTrue(edit.fits(2))
        assertEquals(2, edit.cursor)
    }

    @Test fun samsungSearchHasAFallbackForItsUnreportedLimit() {
        val edit = VerenuInsertionEdit("x".repeat(61), 61, 61, "y".repeat(44))
        val search = "com.sec.android.app.launcher:id/search_src_text"
        assertFalse(edit.fits(-1, search))
        assertTrue(edit.fits(200, search))
        assertFalse(edit.fits(50, search))
        assertTrue(edit.fits(-1, "com.example.editor:id/search_src_text"))
        assertTrue(VerenuInsertionEdit("", 0, 0, "x".repeat(100)).fits(-1, search))
    }
}
