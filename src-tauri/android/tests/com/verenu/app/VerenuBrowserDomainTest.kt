package com.verenu.app

import org.junit.Assert.assertEquals
import org.junit.Test

class VerenuBrowserDomainTest {
    private fun host(raw: String) = VerenuAccessibilityService.hostFromAddressText(raw)

    @Test fun reducesFullUrlsToTheHost() {
        assertEquals("example.com", host("https://www.example.com/path?q=1#top"))
        assertEquals("mail.google.com", host("https://mail.google.com/mail/u/0/"))
    }

    @Test fun acceptsTheBareHostChromeShowsWhenNotEditing() {
        assertEquals("github.com", host("github.com"))
        assertEquals("example.com", host("  Example.com "))
    }

    @Test fun dropsPortsAndCredentials() {
        assertEquals("localhost.test", host("http://user:pw@localhost.test:8080/x"))
    }

    @Test fun ignoresSearchQueriesAndPlaceholders() {
        assertEquals("", host("best pizza near me"))
        assertEquals("", host("Search or type URL"))
        assertEquals("", host(""))
        assertEquals("", host("localhost"))
    }
}
