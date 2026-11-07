package com.verenu.app

import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test

class VerenuCredentialConsentTest {
    @Test fun credentialWritesRequireConsentButRemovalDoesNot() {
        var writes = 0
        try {
            writeCredentialWithConsent("synthetic", { false }) { writes++ }
            fail("Expected consent_required")
        } catch (error: IllegalStateException) {
            assertEquals("consent_required", error.message)
        }
        assertEquals(0, writes)
        writeCredentialWithConsent("synthetic", { true }) { writes++ }
        writeCredentialWithConsent("", { false }) { writes++ }
        assertEquals(2, writes)
    }
}
