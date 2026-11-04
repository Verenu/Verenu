package com.verenu.app

import java.io.File
import java.nio.file.Files

internal object VerenuPendingDeletion {
    private const val MARKER = "legal-deletion-pending"
    private const val ORIGIN = "legal-deletion-origin-pid"
    private val retained = setOf(MARKER, ORIGIN, "legal-android-wipe-complete", "legal-local-wipe-complete", "legal-remote-delete-confirmed", "lib")

    fun recordOrigin(root: File, pid: Int) {
        File(root, ORIGIN).writeText(pid.toString())
    }

    fun wipeBeforeWebView(root: File, pid: Int): Boolean {
        if (!File(root, MARKER).isFile) return false
        if (File(root, "legal-android-wipe-complete").isFile) return true
        val origin = File(root, ORIGIN)
        val originPid = origin.readTextOrNull()?.trim()?.toIntOrNull()
            ?: error("Deletion origin is unavailable")
        if (originPid == pid) return false
        root.listFiles()?.forEach { entry ->
            when {
                entry.name in retained -> Unit
                entry.name == "shared_prefs" && entry.isDirectory && !Files.isSymbolicLink(entry.toPath()) -> {
                    entry.listFiles()?.filter { it.name != "verenu_secrets.xml" }?.forEach(::remove)
                }
                else -> remove(entry)
            }
        } ?: error("Cannot list application data")
        File(root, "legal-android-wipe-complete").outputStream().use { stream ->
            stream.write(1)
            stream.fd.sync()
        }
        if (origin.exists()) check(origin.delete()) { "Cannot remove deletion origin" }
        return true
    }

    private fun File.readTextOrNull(): String? = try { readText() } catch (_: java.io.IOException) { null }

    private fun remove(file: File) {
        if (file.isDirectory && !Files.isSymbolicLink(file.toPath())) {
            file.listFiles()?.forEach(::remove) ?: error("Cannot list application data")
        }
        check(file.delete()) { "Cannot remove application data" }
    }
}
