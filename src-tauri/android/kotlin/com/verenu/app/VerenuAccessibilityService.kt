package com.verenu.app

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.AccessibilityServiceInfo
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.PixelFormat
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.provider.Settings
import android.util.Log
import android.view.Gravity
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.Toast

/**
 * Verenu's Android integration point. Replaces the desktop hotkey + clipboard
 * paste + pill window with OS-native equivalents:
 *
 * - Detects IME visibility (exact callbacks on API 33+, editable-focus
 *   heuristic below) and shows [VerenuOverlayView] as a
 *   `TYPE_ACCESSIBILITY_OVERLAY` window. The overlay appears ONLY while a
 *   keyboard is open over an editable field and disappears when it closes —
 *   there is never a permanent bubble, it never takes focus
 *   (`FLAG_NOT_FOCUSABLE`), and it never replaces the user's keyboard.
 * - Top-anchored below the status bar by design: keyboard height is not
 *   queryable from a service on all supported APIs, so anchoring above the
 *   IME would occlude either the field or the keyboard on some devices. The
 *   pill stays clear of both everywhere from API 26 to 34.
 * - Forwards the foreground package to Rust for Android Context labels
 *   (package name only — never field contents, URLs, or notifications).
 * - Performs insertion: direct `ACTION_SET_TEXT` with cursor restoration
 *   where the app allows it, clipboard fallback (plus optional `ACTION_PASTE`)
 *   where blocked, then acks Rust over the loopback bridge.
 * - Pushes Keystore-unlocked credentials into Rust at connect and drains
 *   staged rotations from the bridge poll loop.
 *
 * Only Android SDK APIs are used — no Tauri imports — so this compiles in
 * the generated `gen/android` project untouched by CLI upgrades. First-SDK-
 * build verification checklist lives in `src-tauri/android/README.md`.
 */
class VerenuAccessibilityService : AccessibilityService(), VerenuOverlayView.Listener {

    companion object {
        const val TAG = "VerenuA11y"
        const val POLL_VISIBLE_MS = 250L
        const val POLL_IDLE_MS = 2000L
        const val INSERT_RETRY_MS = 2000L
        const val PENDING_MAX_AGE_MS = 60_000L
        const val TRANSIENT_AUTO_HIDE_MS = 10_000L

        /** Used by future Settings UI to deep-link recovery correctly. */
        fun isServiceEnabled(context: Context): Boolean {
            val expected = "${context.packageName}/${VerenuAccessibilityService::class.java.name}"
            val enabled = Settings.Secure.getString(
                context.contentResolver,
                Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
            ) ?: return false
            return enabled.split(':').any { it.equals(expected, ignoreCase = true) }
        }
    }

    private lateinit var bridge: VerenuBridge
    private lateinit var keystore: VerenuKeystore
    private lateinit var windowManager: WindowManager
    private var overlay: VerenuOverlayView? = null
    private var overlayAttached = false

    private var worker: HandlerThread? = null
    private var poller: Handler? = null

    @Volatile private var keyboardVisible = false
    @Volatile private var hasEditableFocus = false
    @Volatile private var foregroundPackage = ""
    @Volatile private var supportsSetText = false

    @Volatile private var overlayState = VerenuOverlayView.State.IDLE
    @Volatile private var lastSeenPendingSeq = -1L
    @Volatile private var lastInsertAttemptMs = 0L
    @Volatile private var transientShownAtMs = 0L
    @Volatile private var imeCallbacksArmed = false

    private var showCallback: Any? = null
    private var hideCallback: Any? = null

    // ------------------------------------------------------------------ setup

    override fun onServiceConnected() {
        bridge = VerenuBridge(this)
        windowManager = getSystemService(WINDOW_SERVICE) as WindowManager
        try {
            keystore = VerenuKeystore(this)
            keystore.pushAll(bridge)
        } catch (e: Exception) {
            Log.e(TAG, "keystore unavailable at connect", e)
        }
        armImeCallbacks()
        worker = HandlerThread("verenu-bridge-poll").apply { start() }
        poller = Handler(worker!!.looper)
        schedulePoll(0L)
        Log.i(TAG, "connected")
    }

    /**
     * Exact keyboard show/hide on API 33+. Below that (or if the controller
     * is unavailable) the focus heuristic in [onAccessibilityEvent] drives
     * the overlay. Wrapped defensively: a failure here must never disable
     * the service, it only costs exactness on old devices.
     */
    private fun armImeCallbacks() {
        if (Build.VERSION.SDK_INT < 33 || imeCallbacksArmed) return
        try {
            val controller = softKeyboardController ?: return
            val show = object : android.accessibilityservice.AccessibilityService.SoftKeyboardController.OnShowCallback {
                override fun onShown(controller: android.accessibilityservice.AccessibilityService.SoftKeyboardController) {
                    onKeyboardChanged(true)
                }
            }
            val hide = object : android.accessibilityservice.AccessibilityService.SoftKeyboardController.OnHideCallback {
                override fun onHidden(controller: android.accessibilityservice.AccessibilityService.SoftKeyboardController) {
                    onKeyboardChanged(false)
                }
            }
            controller.addOnShowCallback(mainExecutor, show)
            controller.addOnHideCallback(mainExecutor, hide)
            showCallback = show
            hideCallback = hide
            imeCallbacksArmed = true
        } catch (t: Throwable) {
            Log.w(TAG, "soft-keyboard callbacks unavailable; using focus heuristic", t)
        }
    }

    override fun onUnbind(intent: Intent?): Boolean {
        poller?.removeCallbacksAndMessages(null)
        worker?.quitSafely()
        worker = null
        hideOverlay()
        bridge.invalidate()
        return super.onUnbind(intent)
    }

    override fun onInterrupt() {
        hideOverlay()
    }

    // --------------------------------------------------------------- events

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        if (event == null) return
        val pkg = event.packageName?.toString() ?: ""
        if (pkg.isNotEmpty() && pkg != packageName) {
            foregroundPackage = pkg
        }
        when (event.eventType) {
            AccessibilityEvent.TYPE_VIEW_FOCUSED -> {
                val node = event.source
                try {
                    val editable = node?.isEditable == true
                    hasEditableFocus = editable
                    supportsSetText = editable &&
                        node?.actionList?.any { it.id == AccessibilityNodeInfo.ACTION_SET_TEXT } == true
                    if (editable && imeCallbacksArmed.not()) {
                        // Pre-33 heuristic: editable focus implies the keyboard.
                        onKeyboardChanged(true)
                    }
                    refreshOverlayVisibility()
                } finally {
                    node?.recycle()
                }
            }
            AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED,
            AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED,
            -> {
                // Selection/caret moved — no visibility change, but a pending
                // insertion may now have a valid target.
            }
            AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED -> {
                if (imeCallbacksArmed.not()) {
                    // Leaving to a window without an editable field hides us.
                    val node = rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
                    val editable = node?.isEditable == true
                    node?.recycle()
                    hasEditableFocus = editable
                    if (!editable) onKeyboardChanged(false) else refreshOverlayVisibility()
                }
            }
        }
    }

    private fun onKeyboardChanged(visible: Boolean) {
        if (keyboardVisible == visible && visible) return
        keyboardVisible = visible
        if (!visible) hasEditableFocus = false
        try {
            bridge.postFocus(visible, hasEditableFocus)
        } catch (e: Exception) {
            Log.w(TAG, "focus post failed", e)
        }
        refreshOverlayVisibility()
    }

    // ---------------------------------------------------------------- overlay

    private fun shouldShowOverlay(): Boolean = keyboardVisible && hasEditableFocus

    private fun refreshOverlayVisibility() {
        if (shouldShowOverlay()) {
            showOverlay()
        } else if (!isDictationActive()) {
            hideOverlay()
        }
        // In-flight dictation keeps rendering across keyboard churn; the pill
        // re-anchors when the keyboard returns instead of dropping audio.
    }

    private fun isDictationActive(): Boolean = when (overlayState) {
        VerenuOverlayView.State.RECORDING,
        VerenuOverlayView.State.TRANSCRIBING,
        VerenuOverlayView.State.CLEANING,
        VerenuOverlayView.State.INSERTING,
        -> true
        else -> false
    }

    private fun showOverlay() {
        if (overlayAttached) return
        try {
            val view = VerenuOverlayView(this).apply { listener = this@VerenuAccessibilityService }
            val statusBar = resources.getDimensionPixelSize(
                resources.getIdentifier("status_bar_height", "dimen", "android"),
            )
            val margin = (12 * resources.displayMetrics.density).toInt()
            val params = WindowManager.LayoutParams(
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                    WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
                    WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
                PixelFormat.TRANSLUCENT,
            ).apply {
                gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL
                y = statusBar + margin
            }
            windowManager.addView(view, params)
            overlay = view
            overlayAttached = true
            setOverlayState(VerenuOverlayView.State.IDLE)
        } catch (e: Exception) {
            Log.e(TAG, "cannot attach overlay", e)
        }
    }

    private fun hideOverlay() {
        val view = overlay ?: return
        overlay = null
        overlayAttached = false
        try {
            windowManager.removeView(view)
        } catch (e: Exception) {
            Log.w(TAG, "overlay already detached", e)
        }
    }

    private fun setOverlayState(next: VerenuOverlayView.State) {
        overlayState = next
        if (next == VerenuOverlayView.State.ERROR || next == VerenuOverlayView.State.CANCELLED) {
            transientShownAtMs = System.currentTimeMillis()
        }
        overlay?.setState(next)
    }

    // ------------------------------------------------------------- pill taps

    override fun onPillTap() {
        when (overlayState) {
            VerenuOverlayView.State.IDLE,
            VerenuOverlayView.State.CANCELLED,
            -> startDictation()
            VerenuOverlayView.State.RECORDING -> stopDictation()
            VerenuOverlayView.State.ERROR -> retryDictation()
            else -> Unit // transcribing/cleaning/inserting: taps are no-ops
        }
    }

    override fun onPillCancel() {
        bridge.cancelRecording()
        stopDictationService()
        setOverlayState(VerenuOverlayView.State.CANCELLED)
    }

    override fun onPillRetry() {
        val resp = bridge.retryTranscription()
        if (resp?.optBoolean("ok") == true) {
            setOverlayState(VerenuOverlayView.State.TRANSCRIBING)
        } else {
            overlay?.setError("Retry failed — check connection")
        }
    }

    override fun onPillDismiss() {
        if (!isDictationActive()) {
            hideOverlay()
        }
    }

    private fun startDictation() {
        val resp = bridge.startRecording(foregroundPackage, hasEditableFocus, supportsSetText)
        if (resp?.optBoolean("ok") == true) {
            startDictationService()
            setOverlayState(VerenuOverlayView.State.RECORDING)
        } else {
            overlay?.setError("Could not start recording")
        }
    }

    private fun stopDictation() {
        bridge.stopRecording()
        // The pipeline keeps running (transcribe → clean → handoff); the
        // poll loop advances the pill through those stages.
        setOverlayState(VerenuOverlayView.State.TRANSCRIBING)
    }

    private fun retryDictation() = onPillRetry()

    private fun startDictationService() {
        try {
            val intent = Intent(this, VerenuDictationService::class.java)
                .setAction(VerenuDictationService.ACTION_ACTIVE)
            if (Build.VERSION.SDK_INT >= 29) {
                startForegroundService(intent)
            } else {
                startService(intent)
            }
        } catch (e: Exception) {
            Log.w(TAG, "cannot start dictation service", e)
        }
    }

    private fun stopDictationService() {
        try {
            stopService(Intent(this, VerenuDictationService::class.java))
        } catch (e: Exception) {
            Log.w(TAG, "cannot stop dictation service", e)
        }
    }

    // -------------------------------------------------------------- insertion

    private fun focusedEditable(): AccessibilityNodeInfo? {
        return try {
            val root = rootInActiveWindow ?: return null
            val focus = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            if (focus != null && focus.isEditable) focus else {
                focus?.recycle()
                null
            }
        } catch (e: Exception) {
            Log.w(TAG, "focus read failed", e)
            null
        }
    }

    /**
     * Insert [text] into the focused field. Direct `ACTION_SET_TEXT` with
     * cursor restoration where the app allows it; clipboard (+`ACTION_PASTE`
     * attempt) where blocked. Returns the strategy + success for the ack.
     * Never touches password content: unreadable fields fall straight
     * through to the clipboard path without probing.
     */
    private fun performInsertion(text: String): Pair<String, Boolean> {
        val node = focusedEditable()
        if (node == null) {
            return Pair("clipboard_fallback", copyToClipboard(text, tryPaste = false))
        }
        try {
            val actions = node.actionList.map { it.id }
            if (!actions.contains(AccessibilityNodeInfo.ACTION_SET_TEXT)) {
                return Pair("clipboard_fallback", copyToClipboard(text, tryPaste = true, node = node))
            }
            val ok = if (node.isPassword) {
                node.performAction(
                    AccessibilityNodeInfo.ACTION_SET_TEXT,
                    Bundle().apply {
                        putCharSequence(
                            AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text,
                        )
                    },
                )
            } else {
                val before = node.text?.toString() ?: ""
                val selStart = node.textSelectionStart.takeIf { it >= 0 } ?: before.length
                val selEnd = node.textSelectionEnd.takeIf { it >= 0 } ?: selStart
                val from = selStart.coerceIn(0, before.length)
                val to = selEnd.coerceIn(0, before.length)
                val merged = before.substring(0, minOf(from, to)) + text +
                    before.substring(maxOf(from, to))
                val set = node.performAction(
                    AccessibilityNodeInfo.ACTION_SET_TEXT,
                    Bundle().apply {
                        putCharSequence(
                            AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, merged,
                        )
                    },
                )
                if (set) {
                    val cursor = (minOf(from, to) + text.length).coerceAtMost(merged.length)
                    node.performAction(
                        AccessibilityNodeInfo.ACTION_SET_SELECTION,
                        Bundle().apply {
                            putInt(
                                AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, cursor,
                            )
                            putInt(
                                AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, cursor,
                            )
                        },
                    )
                }
                set
            }
            if (ok) return Pair("direct_accessibility", true)
            return Pair("clipboard_fallback", copyToClipboard(text, tryPaste = true, node = node))
        } finally {
            node.recycle()
        }
    }

    private fun copyToClipboard(
        text: String,
        tryPaste: Boolean,
        node: AccessibilityNodeInfo? = null,
    ): Boolean {
        return try {
            val clipboard = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("Verenu dictation", text))
            if (tryPaste && node != null) {
                node.performAction(AccessibilityNodeInfo.ACTION_PASTE)
            } else {
                Toast.makeText(this, "Copied — paste manually", Toast.LENGTH_SHORT).show()
                true
            }
        } catch (e: Exception) {
            Log.w(TAG, "clipboard fallback failed", e)
            false
        }
    }

    // -------------------------------------------------------------- poll loop

    private fun schedulePoll(delayMs: Long) {
        poller?.postDelayed({ poll() }, delayMs)
    }

    private fun poll() {
        try {
            val snapshot = bridge.getState()
            if (snapshot != null) {
                onBridgeState(snapshot)
            }
        } catch (e: Exception) {
            Log.w(TAG, "bridge poll failed", e)
        }
        // Transient pills auto-hide like the desktop (10s), and only when
        // nothing is in flight.
        if ((overlayState == VerenuOverlayView.State.ERROR ||
                overlayState == VerenuOverlayView.State.CANCELLED) &&
            !isDictationActive() &&
            System.currentTimeMillis() - transientShownAtMs > TRANSIENT_AUTO_HIDE_MS
        ) {
            hideOverlay()
        }
        schedulePoll(if (overlayAttached) POLL_VISIBLE_MS else POLL_IDLE_MS)
    }

    private fun onBridgeState(snapshot: BridgeStateSnapshot) {
        // Staged Keystore rotation → persist, then confirm by re-pushing.
        if (snapshot.keystorePending) {
            try {
                val pending = bridge.takeKeystorePending()
                if (pending != null) {
                    try {
                        keystore.save(pending.first, pending.second)
                        keystore.pushAll(bridge)
                    } catch (e: Exception) {
                        Log.e(TAG, "keystore rotation failed", e)
                    }
                }
            } catch (e: Exception) {
                Log.w(TAG, "keystore drain failed", e)
            }
        }

        // Pill stage refinement while a dictation is in flight.
        if (isDictationActive()) {
            when (snapshot.pillStage) {
                "cleaning" -> if (overlayState == VerenuOverlayView.State.TRANSCRIBING) {
                    setOverlayState(VerenuOverlayView.State.CLEANING)
                }
                "pasting" -> setOverlayState(VerenuOverlayView.State.INSERTING)
            }
            overlay?.setAudioLevel(snapshot.audioLevel)
            if (!snapshot.dictationActive && overlayState == VerenuOverlayView.State.RECORDING) {
                // Backend left recording without our stop (gate rejection,
                // error): fall back to transcribing/error via lifecycle.
                setOverlayState(VerenuOverlayView.State.TRANSCRIBING)
            }
        }

        // Insertion handoff: attempt only with an editable target, throttle
        // retries, give up after a minute (history already holds the text).
        val pending = snapshot.pendingInsertion
        if (pending != null &&
            pending.seq != lastSeenPendingSeq &&
            pending.ageMs < PENDING_MAX_AGE_MS
        ) {
            lastSeenPendingSeq = pending.seq
        }
        val current = snapshot.pendingInsertion
        if (current != null &&
            hasEditableFocus &&
            current.ageMs < PENDING_MAX_AGE_MS &&
            System.currentTimeMillis() - lastInsertAttemptMs > INSERT_RETRY_MS
        ) {
            lastInsertAttemptMs = System.currentTimeMillis()
            val (strategy, ok) = performInsertion(current.text)
            val ack = bridge.ackInsertion(current.seq, ok, strategy, null, foregroundPackage)
            if (ack?.optBoolean("ok") == true || !ok) {
                stopDictationService()
                if (ok) {
                    setOverlayState(VerenuOverlayView.State.IDLE)
                    if (!shouldShowOverlay()) hideOverlay()
                } else {
                    overlay?.setError("Couldn't insert — copied instead")
                    copyToClipboard(current.text, tryPaste = false)
                }
            }
        } else if (current != null && current.ageMs >= PENDING_MAX_AGE_MS) {
            bridge.ackInsertion(current.seq, false, "clipboard_fallback", "expired", foregroundPackage)
        }
    }

    override fun onServiceInfoChanged(info: AccessibilityServiceInfo?) {
        super.onServiceInfoChanged(info)
    }
}
