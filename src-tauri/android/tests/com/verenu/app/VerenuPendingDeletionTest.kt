package com.verenu.app

import java.nio.file.Files
import org.junit.Assert.*
import org.junit.Test

class VerenuPendingDeletionTest {
    @Test fun wipesOnlyAfterProcessChangeAndPreservesPendingIdentity() {
        val root = Files.createTempDirectory("verenu-delete-test").toFile()
        try {
            root.resolve("legal-deletion-pending").writeText("pending")
            root.resolve("shared_prefs").mkdir()
            root.resolve("shared_prefs/verenu_secrets.xml").writeText("encrypted fixture")
            root.resolve("shared_prefs/other.xml").writeText("private fixture")
            root.resolve("app_webview").mkdir()
            root.resolve("app_webview/storage").writeText("private fixture")
            root.resolve("verenu.db").writeText("private fixture")
            VerenuPendingDeletion.recordOrigin(root, 123)
            assertFalse(VerenuPendingDeletion.wipeBeforeWebView(root, 123))
            assertTrue(root.resolve("verenu.db").exists())
            assertTrue(VerenuPendingDeletion.wipeBeforeWebView(root, 456))
            assertTrue(root.resolve("legal-deletion-pending").exists())
            assertTrue(root.resolve("shared_prefs/verenu_secrets.xml").exists())
            assertFalse(root.resolve("shared_prefs/other.xml").exists())
            assertFalse(root.resolve("app_webview").exists())
            assertFalse(root.resolve("verenu.db").exists())
            assertFalse(root.resolve("legal-deletion-origin-pid").exists())
            root.resolve("app_webview").mkdir()
            root.resolve("app_webview/new-session").writeText("new session")
            assertTrue(VerenuPendingDeletion.wipeBeforeWebView(root, 456))
            assertTrue(root.resolve("app_webview/new-session").exists())
        } finally { root.deleteRecursively() }
    }

    @Test fun refusesToFollowSymlinksOutsideTheDataDirectory() {
        val root = Files.createTempDirectory("verenu-delete-test").toFile()
        val outside = Files.createTempDirectory("verenu-outside-test").toFile()
        try {
            root.resolve("legal-deletion-pending").writeText("pending")
            VerenuPendingDeletion.recordOrigin(root, 123)
            outside.resolve("fixture").writeText("keep")
            Files.createSymbolicLink(root.resolve("external").toPath(), outside.toPath())
            assertTrue(VerenuPendingDeletion.wipeBeforeWebView(root, 456))
            assertTrue(outside.resolve("fixture").exists())
            assertFalse(root.resolve("external").exists())
        } finally { root.deleteRecursively(); outside.deleteRecursively() }
    }
}
