package com.verenu.app

import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit

/** Bounded Activity-owned work; closing settles queued and running requests once. */
internal class VerenuAppInfoWorker {
    private val lock = Any()
    private val pending = HashSet<Request>()
    private val executor = ThreadPoolExecutor(
        2, 2, 0L, TimeUnit.MILLISECONDS, ArrayBlockingQueue(128),
        { task -> Thread(task, "verenu-app-info").apply { isDaemon = true } },
        ThreadPoolExecutor.AbortPolicy(),
    )

    private inner class Request(val work: () -> Unit, val cancelled: () -> Unit) : Runnable {
        override fun run() {
            work()
        }

        fun settle(completion: () -> Unit) {
            val owned = synchronized(lock) { pending.remove(this) }
            if (owned) completion()
        }
    }

    fun <T> submit(work: () -> T, complete: (T) -> Unit, cancelled: () -> Unit) {
        lateinit var request: Request
        request = Request({
            val result = try {
                work()
            } catch (_: Exception) {
                request.settle(cancelled)
                return@Request
            }
            request.settle { complete(result) }
        }, cancelled)
        val accepted = synchronized(lock) {
            if (executor.isShutdown) false else {
                pending.add(request)
                try {
                    executor.execute(request)
                    true
                } catch (_: java.util.concurrent.RejectedExecutionException) {
                    pending.remove(request)
                    false
                }
            }
        }
        if (!accepted) cancelled()
    }

    fun close() {
        val abandoned = synchronized(lock) {
            executor.shutdownNow()
            pending.toList().also { pending.clear() }
        }
        abandoned.forEach { it.cancelled() }
    }
}
