package com.verenu.app

/** Stop starting IPC calls after a slow call has consumed the insertion budget. */
internal class VerenuInsertionBudget(
    private val now: () -> Long,
    private val durationMs: Long = 1_500L,
) {
    private val startedAt = now()
    val remainingMs: Long get() = (durationMs - (now() - startedAt)).coerceAtLeast(0L)
    val expired: Boolean get() = remainingMs == 0L

    enum class Confirmation { LANDED, UNCHANGED, UNAVAILABLE }

    /** A failed refresh invalidates the node. Never retry it or trust cached text. */
    fun confirm(
        before: String,
        expected: String,
        refreshText: () -> String?,
        pause: () -> Unit,
    ): Confirmation {
        repeat(2) { attempt ->
            if (expired) return Confirmation.UNAVAILABLE
            val current = refreshText() ?: return Confirmation.UNAVAILABLE
            if (current == expected) return Confirmation.LANDED
            // Another edit or an editor rewrite makes a second paste unsafe.
            if (current != before) return Confirmation.UNAVAILABLE
            if (attempt == 0 && !expired) pause()
        }
        return if (expired) Confirmation.UNAVAILABLE else Confirmation.UNCHANGED
    }
}
