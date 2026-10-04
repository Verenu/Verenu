package com.verenu.app

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class VerenuAppInfoWorkerTest {
    @Test fun destructionSettlesRunningQueuedAndLaterRequestsOnce() {
        val worker = VerenuAppInfoWorker()
        val started = CountDownLatch(2)
        val release = CountDownLatch(1)
        val finished = CountDownLatch(2)
        val cancelled = AtomicInteger()
        val completed = AtomicInteger()
        val queuedWork = AtomicInteger()
        try {
            repeat(2) {
                worker.submit({
                    started.countDown()
                    try {
                        release.await(5, TimeUnit.SECONDS)
                    } catch (_: InterruptedException) {
                        // Model a platform call finishing after shutdown.
                    } finally {
                        finished.countDown()
                    }
                }, { completed.incrementAndGet() }, { cancelled.incrementAndGet() })
            }
            assertTrue(started.await(5, TimeUnit.SECONDS))
            worker.submit({ queuedWork.incrementAndGet() }, { completed.incrementAndGet() }, { cancelled.incrementAndGet() })
            worker.close()
            worker.close()
            worker.submit({ queuedWork.incrementAndGet() }, { completed.incrementAndGet() }, { cancelled.incrementAndGet() })
            assertTrue(finished.await(5, TimeUnit.SECONDS))
            assertEquals(4, cancelled.get())
            assertEquals(0, completed.get())
            assertEquals(0, queuedWork.get())
        } finally {
            release.countDown()
            worker.close()
        }
    }

    @Test fun completedRequestIsNotCancelledOnDestruction() {
        val worker = VerenuAppInfoWorker()
        val completed = CountDownLatch(1)
        val cancelled = AtomicInteger()
        try {
            worker.submit({ 42 }, { assertEquals(42, it); completed.countDown() }, { cancelled.incrementAndGet() })
            assertTrue(completed.await(5, TimeUnit.SECONDS))
            worker.close()
            assertEquals(0, cancelled.get())
        } finally {
            worker.close()
        }
    }
}
