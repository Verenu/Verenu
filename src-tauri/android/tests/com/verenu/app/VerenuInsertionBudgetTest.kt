package com.verenu.app

import org.junit.Assert.assertEquals
import org.junit.Test

class VerenuInsertionBudgetTest {
    @Test fun successfulInsertionIsConfirmedImmediately() {
        val budget = VerenuInsertionBudget({ 0L })
        assertEquals(VerenuInsertionBudget.Confirmation.LANDED,
            budget.confirm("Hello", "Hello there", { "Hello there" }, { error("unexpected retry") }))
    }

    @Test fun delayedEditorUpdateIsConfirmed() {
        var text = "Hello"
        val budget = VerenuInsertionBudget({ 0L })
        assertEquals(VerenuInsertionBudget.Confirmation.LANDED,
            budget.confirm("Hello", "Hello there", { text }, { text = "Hello there" }))
    }

    @Test fun timedOutActionDoesNotStartAnyReadback() {
        var time = 0L
        val budget = VerenuInsertionBudget({ time })
        time += 5_000L // Android's action IPC timeout.
        assertEquals(VerenuInsertionBudget.Confirmation.UNAVAILABLE,
            budget.confirm("", "Hello", { error("readback after timeout") }, { error("retry after timeout") }))
    }

    @Test fun timedOutRefreshDoesNotRepeatFiveSecondWait() {
        var time = 0L
        var refreshes = 0
        val budget = VerenuInsertionBudget({ time })
        assertEquals(VerenuInsertionBudget.Confirmation.UNAVAILABLE,
            budget.confirm("", "Hello", {
                refreshes++
                time += 5_000L
                null
            }, { error("retry on stale node") }))
        assertEquals(1, refreshes)
    }

    @Test fun cachedTextOnInvalidNodeCannotConfirmSuccess() {
        val budget = VerenuInsertionBudget({ 0L })
        assertEquals(VerenuInsertionBudget.Confirmation.UNAVAILABLE,
            budget.confirm("", "Hello", { null }, { error("retry on stale node") }))
    }

    @Test fun existingOccurrenceDoesNotConfirmSecondInsertion() {
        val budget = VerenuInsertionBudget({ 0L })
        assertEquals(VerenuInsertionBudget.Confirmation.UNCHANGED,
            budget.confirm("Hello", "Hello Hello", { "Hello" }, {}))
    }

    @Test fun editorRewriteDoesNotAuthorizeDuplicatePaste() {
        val budget = VerenuInsertionBudget({ 0L })
        assertEquals(VerenuInsertionBudget.Confirmation.UNAVAILABLE,
            budget.confirm("", "Hello there", { "Hello there\n" }, { error("retry after rewrite") }))
    }
}
