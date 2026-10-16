package com.verenu.app

internal class VerenuInsertionBudget(
    private val now: () -> Long,
    private val durationMs: Long = 1_500L,
) {
    private val startedAt = now()
    val remainingMs: Long get() = (durationMs - (now() - startedAt)).coerceAtLeast(0L)
    val expired: Boolean get() = remainingMs == 0L

    enum class Confirmation { LANDED, UNCHANGED, UNAVAILABLE }

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
            if (current != before) return Confirmation.UNAVAILABLE
            if (attempt == 0 && !expired) pause()
        }
        return if (expired) Confirmation.UNAVAILABLE else Confirmation.UNCHANGED
    }
}
