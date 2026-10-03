package com.verenu.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VerenuFocusPackagesTest {
    private val app = "com.verenu.app"
    private val ime = "com.google.android.inputmethod.latin"
    private val launcher = "com.sec.android.app.launcher"

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
