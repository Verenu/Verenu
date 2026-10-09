package com.verenu.app

/** Polls while muting, or until a failed restore has succeeded. */
internal class VerenuMediaMuteLifecycle(private val apply: (Boolean) -> Boolean) {
    private var requested = false
    val pollDelayMs: Long get() = if (requested) 100L else 1000L

    fun update(muted: Boolean): Boolean {
        requested = muted
        return poll()
    }

    fun poll(): Boolean {
        val succeeded = apply(requested)
        return requested || !succeeded
    }
}
