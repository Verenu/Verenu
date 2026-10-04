package com.verenu.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VerenuFocusPackagesTest {
    private val app = "com.verenu.app"
    private val ime = "com.google.android.inputmethod.latin"
    private val launcher = "com.sec.android.app.launcher"

    @Test fun unfocusedSplitScreenContentCannotReplaceTarget() {
        val editor = "com.example.editor"
        assertTrue(VerenuFocusPackages.eventTarget(editor, "com.android.chrome", false, app, ime) == editor)
    }

    @Test fun focusingOtherSplitScreenAppChangesTarget() {
        assertTrue(VerenuFocusPackages.eventTarget(launcher, "com.android.chrome", true, app, ime) == "com.android.chrome")
    }

    @Test fun keyboardAndOverlayFocusPreserveAppTarget() {
        for (owner in listOf(ime, app, "android", "com.android.systemui")) {
            assertTrue(VerenuFocusPackages.eventTarget(launcher, owner, true, app, ime) == launcher)
        }
    }

    @Test fun privacyIndicatorCannotReplaceLauncherTarget() {
        var foreground = launcher
        for (eventOwner in listOf("com.android.systemui", ime, app, "android", "")) {
            if (VerenuFocusPackages.isAppWindow(eventOwner, app, ime)) foreground = eventOwner
        }
        assertTrue(foreground == launcher)
    }

    @Test fun focusedSearchCanReceiveTextAfterSystemUiWindowEvents() {
        assertTrue(VerenuFocusPackages.isInsertionField(launcher, ime))
        assertFalse(VerenuFocusPackages.isInsertionField("com.android.systemui", ime))
    }

    @Test fun keyboardSearchIsNotAnAppInsertionTarget() {
        assertFalse(VerenuFocusPackages.isInsertionField(ime, ime))
    }

    @Test fun appWindowChangeStillUpdatesTheContextTarget() {
        assertTrue(VerenuFocusPackages.isAppWindow("com.example.editor", app, ime))
    }

    @Test fun ownEditableFieldsRemainAvailableForInsertion() {
        assertTrue(VerenuFocusPackages.isInsertionField(app, ime))
    }
}
