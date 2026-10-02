package com.verenu.app

import android.accessibilityservice.AccessibilityService
import android.animation.ValueAnimator
import android.app.KeyguardManager
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.PixelFormat
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
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
 * - Detects IME visibility with an editable-focus plus default-IME window
 *   event heuristic (Android exposes no show/hide callback to an
 *   AccessibilityService) and shows [VerenuOverlayView] as a
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
        const val BACKEND_LAUNCH_COOLDOWN_MS = 20_000L
        const val BACKEND_START_WAIT_MS = 12_000L
        const val HIDE_DEBOUNCE_MS = 450L
        const val POLL_RECORDING_MS = 60L
        const val INSERT_ATTEMPT_MS = 350L
        const val INSERT_WAIT_FOR_FIELD_MS = 6_000L
        const val INSERT_GIVE_UP_MS = 12_000L

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
    private var requestWorker: HandlerThread? = null
    private var requester: Handler? = null
    private val mainHandler = Handler(Looper.getMainLooper())
    private val imeVisibilityCheck = Runnable { refreshKeyboardVisibilityFromWindows() }

    @Volatile private var keyboardVisible = false
    @Volatile private var hasEditableFocus = false
    @Volatile private var foregroundPackage = ""
    @Volatile private var supportsSetText = false
    @Volatile private var deviceLocked = false

    @Volatile private var overlayState = VerenuOverlayView.State.IDLE
    @Volatile private var overlayErrorMessage = ""
    private enum class ErrorAction {
        RETRY_START,
        RETRY_STOP,
        RETRY_CANCEL,
        RETRY_TRANSCRIPTION,
        RETRY_INSERTION,
        DISMISS,
    }
    @Volatile private var errorAction = ErrorAction.RETRY_TRANSCRIPTION
    @Volatile private var stopRequestInFlight = false
    @Volatile private var cancelRequestInFlight = false
    @Volatile private var lastSeenPendingSeq = -1L
    @Volatile private var lastInsertAttemptMs = 0L
    @Volatile private var insertionStartedAtMs = 0L
    // ACTION_SET_TEXT is not idempotent: if the insertion succeeds but the
    // follow-up ack packet is lost, repeating it would duplicate the dictated
    // text. Remember the successful handoff and retry only the ack until Rust
    // clears the outbox.
    @Volatile private var lastInsertedSeq = -1L
    @Volatile private var lastInsertedStrategy = "direct_accessibility"
    @Volatile private var lastInsertedPackage = ""
    @Volatile private var lastInsertionAckAttemptMs = 0L
    @Volatile private var transientShownAtMs = 0L
    @Volatile private var currentRunId = ""
    @Volatile private var recordingStartedAtMs = 0L
    @Volatile private var lastPipelineStage = ""
    @Volatile private var lastBackendLaunchMs = 0L
    private var overlayParams: WindowManager.LayoutParams? = null
    private var overlayMoveAnimator: ValueAnimator? = null
    @Volatile private var pillPosition = "keyboard-center"
    @Volatile private var appearanceMode = "system"
    @Volatile private var backendWasUp = false
    // ------------------------------------------------------------------ setup

    override fun onServiceConnected() {
        bridge = VerenuBridge(this)
        windowManager = getSystemService(WINDOW_SERVICE) as WindowManager
        deviceLocked = isDeviceLocked()
        try {
            keystore = VerenuKeystore(this)
        } catch (e: Exception) {
            Log.e(TAG, "keystore unavailable at connect", e)
        }
        worker = HandlerThread("verenu-bridge-poll").apply { start() }
        poller = Handler(worker!!.looper)
        requestWorker = HandlerThread("verenu-bridge-request").apply { start() }
        requester = Handler(requestWorker!!.looper)
        // Credential hydration performs authenticated loopback I/O. Keep it
        // off Android's accessibility callback thread; Android will throw
        // NetworkOnMainThreadException here on recent releases.
        if (deviceLocked) {
            requester?.post { bridge.clearCredentials() }
        } else {
            hydrateCredentials()
        }
        schedulePoll(0L)
        requester?.post { ensureBackendRunning(waitMs = 0L) }
        Log.i(TAG, "connected")
    }

    override fun onUnbind(intent: Intent?): Boolean {
        mainHandler.removeCallbacks(imeVisibilityCheck)
        poller?.removeCallbacksAndMessages(null)
        requester?.removeCallbacksAndMessages(null)
        // Process this after removing queued requests, but before the worker
        // is asked to quit. This is best-effort because Android may tear down
        // a disabled service immediately; lock transitions are also covered
        // by syncDeviceLockState() below.
        requester?.post {
            if (::bridge.isInitialized) {
                bridge.clearCredentials()
                bridge.invalidate()
            }
        }
        worker?.quitSafely()
        requestWorker?.quitSafely()
        worker = null
        requestWorker = null
        hideOverlay()
        if (::bridge.isInitialized) bridge.invalidate()
        return super.onUnbind(intent)
    }

    override fun onInterrupt() {
        mainHandler.removeCallbacks(imeVisibilityCheck)
        hideOverlay()
    }

    // --------------------------------------------------------------- events

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        if (event == null) return
        val pkg = event.packageName?.toString() ?: ""
        val imePackage = defaultImePackage()
        if (Log.isLoggable(TAG, Log.DEBUG)) {
            Log.d(TAG, "event type=${AccessibilityEvent.eventTypeToString(event.eventType)} pkg=$pkg ime=$imePackage")
        }
        // IME events describe the keyboard, not the app being edited. Never
        // let Gboard/Samsung Keyboard become the Context or insertion target.
        if (pkg.isNotEmpty() && pkg != packageName && pkg != imePackage) {
            foregroundPackage = pkg
        }
        when (event.eventType) {
            AccessibilityEvent.TYPE_VIEW_FOCUSED -> {
                val node = event.source
                try {
                    val editable = node?.isEditable == true && node.isPassword.not()
                    hasEditableFocus = editable
                    supportsSetText = editable &&
                        node?.actionList?.any { it.id == AccessibilityNodeInfo.ACTION_SET_TEXT } == true
                    // Android exposes no reliable keyboard show/hide callback
                    // to AccessibilityService. Editable focus starts the
                    // heuristic; IME window events below keep it current.
                    if (editable) onKeyboardChanged(true)
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
                // IME packages emit window-state changes when their surface
                // opens/closes. Restrict this to the configured default IME;
                // other app windows must never make the overlay permanent.
                if (pkg.isNotEmpty() && pkg == imePackage) {
                    onKeyboardChanged(true)
                } else if (pkg != packageName) {
                    // Our own overlay window also produces window-state
                    // events and must not clear real focus. For any other app
                    // window only refresh the editable-focus flag: whether the
                    // keyboard is actually up is decided from the live window
                    // list in the check below. Flipping it here made the pill
                    // blink whenever a system window (one-hand mode, the edge
                    // panel, a toast) came and went.
                    val node = findInputFocus()
                    val editable = node?.isEditable == true && node.isPassword.not()
                    node?.recycle()
                    if (editable || !keyboardVisible) hasEditableFocus = editable
                }
            }
        }
        // Focus and IME window events are delivered independently. Re-check
        // the actual interactive window list after the event settles so a
        // keyboard close cannot leave a stale error/idle pill behind.
        scheduleKeyboardVisibilityCheck()
    }

    private fun scheduleKeyboardVisibilityCheck() {
        mainHandler.removeCallbacks(imeVisibilityCheck)
        mainHandler.postDelayed(imeVisibilityCheck, 100L)
    }

    private fun defaultImePackage(): String? = Settings.Secure.getString(
        contentResolver,
        Settings.Secure.DEFAULT_INPUT_METHOD,
    )?.substringBefore('/')

    private fun isDeviceLocked(): Boolean =
        (getSystemService(KEYGUARD_SERVICE) as? KeyguardManager)?.isKeyguardLocked == true

    private fun hydrateCredentials() {
        requester?.post {
            try {
                if (::keystore.isInitialized && !isDeviceLocked()) keystore.pushAll(bridge)
            } catch (e: Exception) {
                Log.e(TAG, "keystore hydration failed", e)
            }
        }
    }

    private fun syncDeviceLockState() {
        val locked = isDeviceLocked()
        if (locked == deviceLocked) return
        deviceLocked = locked
        if (locked) {
            // Stop capture first, then clear every transient secret held by
            // Rust. The service's durable Keystore copy remains protected by
            // Android's encrypted storage and is rehydrated after unlock.
            runOnBridge {
                bridge.cancelRecording()
                bridge.clearCredentials()
            }
            mainHandler.post {
                stopDictationService()
                overlayErrorMessage = ""
                setOverlayState(VerenuOverlayView.State.IDLE)
                hideOverlay()
            }
        } else {
            hydrateCredentials()
        }
    }

    /**
     * The input-focused node of the active window. WebViews (Chrome, in-app
     * browsers, Verenu's own UI) often don't report input focus through
     * [AccessibilityNodeInfo.findFocus], so fall back to a bounded walk for a
     * focused editable node. Callers own (and recycle) the result.
     */
    private fun findInputFocus(): AccessibilityNodeInfo? {
        val root = rootInActiveWindow ?: return null
        val direct = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
        if (direct != null && direct.isEditable) return direct
        direct?.recycle()
        var budget = 600
        fun walk(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
            if (budget-- <= 0) return null
            if (node.isFocused && node.isEditable) return AccessibilityNodeInfo.obtain(node)
            for (i in 0 until node.childCount) {
                val child = node.getChild(i) ?: continue
                val found = walk(child)
                child.recycle()
                if (found != null) return found
            }
            return null
        }
        return walk(root)
    }

    private fun refreshKeyboardVisibilityFromWindows() {
        val imeVisible = try {
            // AccessibilityWindowInfo.isActive is false for Samsung's IME
            // even while it is on-screen. Presence of the IME window is the
            // useful signal here; the service polls briefly through the close
            // animation because Samsung may omit the final event entirely.
            windows.any {
                it.type == android.view.accessibility.AccessibilityWindowInfo.TYPE_INPUT_METHOD
            }
        } catch (e: Exception) {
            Log.w(TAG, "IME window state unavailable", e)
            return
        }
        if (imeVisible && (!keyboardVisible || !hasEditableFocus)) {
            // The IME can open (or reopen on an already-focused field) without
            // a fresh view-focus event, so also re-probe when the keyboard is
            // known to be up but no editable focus is.
            val node = findInputFocus()
            val editable = node?.isEditable == true && node.isPassword.not()
            supportsSetText = editable &&
                node?.actionList?.any { it.id == AccessibilityNodeInfo.ACTION_SET_TEXT } == true
            node?.recycle()
            if (editable) {
                hasEditableFocus = true
                if (!keyboardVisible) {
                    onKeyboardChanged(true)
                } else {
                    runOnBridge { bridge.postFocus(true, true) }
                    refreshOverlayVisibility()
                }
            }
        } else if (!imeVisible && !isDictationActive()) {
            // Keep the visibility invariant true even if an OEM omits the
            // final IME event during a close animation.
            if (keyboardVisible) onKeyboardChanged(false) else refreshOverlayVisibility()
        }
        if (keyboardVisible) {
            if (overlayAttached) applyOverlayPresentation() else refreshOverlayVisibility()
            mainHandler.postDelayed(imeVisibilityCheck, 60L)
        }
    }

    private fun onKeyboardChanged(visible: Boolean) {
        if (Log.isLoggable(TAG, Log.DEBUG)) Log.d(TAG, "keyboardChanged visible=$visible editable=$hasEditableFocus")
        if (keyboardVisible == visible && visible) return
        keyboardVisible = visible
        if (!visible) hasEditableFocus = false
        runOnBridge { bridge.postFocus(visible, hasEditableFocus) }
        refreshOverlayVisibility()
    }

    /**
     * Rust (and therefore the loopback bridge) lives in the Tauri activity's
     * process. When Android restarts this process just to bind the service
     * (boot, update, crash), nothing has loaded Rust yet, so start the
     * activity in background mode and optionally wait for the bridge. Runs on
     * the requester thread.
     */
    private fun ensureBackendRunning(waitMs: Long): Boolean {
        if (bridge.getState() != null) return true
        val now = System.currentTimeMillis()
        if (now - lastBackendLaunchMs > BACKEND_LAUNCH_COOLDOWN_MS) {
            lastBackendLaunchMs = now
            try {
                startActivity(
                    Intent(this, MainActivity::class.java)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                        .putExtra(MainActivity.EXTRA_BACKGROUND_START, true),
                )
            } catch (e: Exception) {
                Log.w(TAG, "cannot start backend", e)
            }
        }
        val deadline = now + waitMs
        while (System.currentTimeMillis() < deadline) {
            try {
                Thread.sleep(300L)
            } catch (_: InterruptedException) {
                return false
            }
            if (bridge.getState() != null) return true
        }
        return false
    }

    /** All loopback I/O stays off Android's main thread. */
    private fun runOnBridge(block: () -> Unit) {
        val handler = requester
        if (handler == null) {
            Log.w(TAG, "bridge worker unavailable")
            return
        }
        handler.post {
            try {
                block()
            } catch (e: Exception) {
                Log.w(TAG, "bridge request failed", e)
            }
        }
    }

    // ---------------------------------------------------------------- overlay

    /**
     * The pill shows while a keyboard is up over an editable field, and also
     * for as long as a dictation is in flight (even with the keyboard closed,
     * docked) or an error/cancel notice is still fresh.
     */
    private fun shouldShowOverlay(): Boolean =
        (keyboardVisible && hasEditableFocus) || isDictationActive() || transientNoticeVisible()

    private fun transientNoticeVisible(): Boolean =
        (overlayState == VerenuOverlayView.State.ERROR ||
            overlayState == VerenuOverlayView.State.CANCELLED) &&
            System.currentTimeMillis() - transientShownAtMs < TRANSIENT_AUTO_HIDE_MS

    private fun refreshOverlayVisibility() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            mainHandler.post { refreshOverlayVisibility() }
            return
        }
        if (shouldShowOverlay()) {
            mainHandler.removeCallbacks(hideOverlayRunnable)
            showOverlay()
            applyOverlayPresentation()
        } else if (overlayAttached) {
            // Debounce: IME and focus events routinely bounce for a moment
            // (focus moves between fields, Samsung emits transient window
            // changes). Hiding instantly made the pill blink off and back on.
            mainHandler.removeCallbacks(hideOverlayRunnable)
            mainHandler.postDelayed(hideOverlayRunnable, HIDE_DEBOUNCE_MS)
        }
    }

    private val hideOverlayRunnable = Runnable {
        if (!shouldShowOverlay()) hideOverlay()
    }

    private fun isDictationActive(): Boolean = when (overlayState) {
        VerenuOverlayView.State.RECORDING,
        VerenuOverlayView.State.TRANSCRIBING,
        VerenuOverlayView.State.CLEANING,
        VerenuOverlayView.State.INSERTING,
        -> true
        else -> false
    }

    /**
     * The front camera's hole-punch bounds in absolute screen pixels, when the
     * device reports one (centered on cover screens, in a corner on others).
     */
    private fun cutoutRect(): android.graphics.Rect? {
        if (Build.VERSION.SDK_INT < 28) return null
        val cutout = try {
            @Suppress("DEPRECATION")
            windowManager.defaultDisplay?.cutout
        } catch (e: Exception) {
            null
        } ?: return null
        return cutout.boundingRects.firstOrNull { !it.isEmpty }
    }

    private fun statusBarHeightPx(): Int = resources.getDimensionPixelSize(
        resources.getIdentifier("status_bar_height", "dimen", "android"),
    )

    /** Whether the placement follows the keyboard (as opposed to the screen). */
    private fun followsKeyboard() = pillPosition.startsWith("keyboard-")

    /** Docked: a dictation continues but there is no keyboard to sit on. */
    private fun isDocked(): Boolean =
        isDictationActive() && !(keyboardVisible && hasEditableFocus && imeTopPx() != null)

    /**
     * Fill in gravity/offsets for the chosen placement. `x` is an offset from
     * the gravity's anchor (the window center for CENTER_HORIZONTAL).
     */
    private fun placeOverlay(params: WindowManager.LayoutParams) {
        val density = resources.displayMetrics.density
        val margin = (12 * density).toInt()
        val docked = isDocked()
        val imeTop = if (keyboardVisible) imeTopPx() else null
        val screenWidth = resources.displayMetrics.widthPixels
        val cutout = cutoutRect()

        fun top(offset: Int = 0) {
            params.gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL
            params.x = offset
            params.y = statusBarHeightPx() + (8 * density).toInt()
        }

        fun underPunchHole() {
            if (cutout != null) {
                params.gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL
                // Offset from center so the pill sits under an off-center camera.
                params.x = cutout.centerX() - screenWidth / 2
                params.y = cutout.bottom + (4 * density).toInt()
            } else {
                top()
            }
        }

        when {
            // Docked while the keyboard is away: out of the way, at the top.
            docked && followsKeyboard() -> underPunchHole()
            pillPosition == "screen-top" -> top()
            pillPosition == "screen-middle" -> {
                params.gravity = Gravity.CENTER
                params.x = 0
                params.y = 0
            }
            pillPosition == "punch-hole" -> underPunchHole()
            imeTop != null -> {
                val above = (realScreenHeightPx() - imeTop + (8 * density).toInt()).coerceAtLeast(0)
                params.y = above
                when (pillPosition) {
                    "keyboard-left" -> {
                        params.gravity = Gravity.BOTTOM or Gravity.START
                        params.x = margin
                    }
                    "keyboard-right" -> {
                        params.gravity = Gravity.BOTTOM or Gravity.END
                        params.x = margin
                    }
                    else -> {
                        params.gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
                        params.x = 0
                    }
                }
            }
            else -> underPunchHole()
        }
    }

    /** Push theme/compact state into the view and glide it to its placement. */
    private fun applyOverlayPresentation() {
        val view = overlay ?: return
        view.setDark(resolveDark())
        view.setCompact(isDocked())
        repositionOverlay()
    }

    private fun resolveDark(): Boolean = when (appearanceMode) {
        "dark" -> true
        "light" -> false
        else -> (resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES
    }

    /** Keep the pill glued to its anchor, easing between positions. */
    private fun repositionOverlay() {
        val view = overlay ?: return
        val params = overlayParams ?: return
        val fromY = params.y
        val fromX = params.x
        val fromGravity = params.gravity
        placeOverlay(params)
        val toY = params.y
        val toX = params.x
        if (fromGravity != params.gravity) {
            // Anchor change (e.g. keyboard → docked): jump the anchor, then
            // let the pill fade in from its new spot rather than slide across.
            applyLayout(view, params)
            return
        }
        if (fromY == toY && fromX == toX) {
            params.y = toY
            params.x = toX
            return
        }
        overlayMoveAnimator?.cancel()
        overlayMoveAnimator = ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 140
            addUpdateListener {
                val f = it.animatedValue as Float
                params.y = (fromY + (toY - fromY) * f).toInt()
                params.x = (fromX + (toX - fromX) * f).toInt()
                applyLayout(view, params)
            }
            start()
        }
    }

    private fun applyLayout(view: VerenuOverlayView, params: WindowManager.LayoutParams) {
        try {
            if (overlayAttached && overlay === view) windowManager.updateViewLayout(view, params)
        } catch (e: Exception) {
            Log.w(TAG, "cannot move overlay", e)
        }
    }

    private fun imeTopPx(): Int? = try {
        val rect = android.graphics.Rect()
        windows
            .firstOrNull { it.type == android.view.accessibility.AccessibilityWindowInfo.TYPE_INPUT_METHOD }
            ?.let {
                it.getBoundsInScreen(rect)
                if (rect.height() > 0) rect.top else null
            }
    } catch (e: Exception) {
        null
    }

    private fun realScreenHeightPx(): Int =
        if (Build.VERSION.SDK_INT >= 30) {
            windowManager.currentWindowMetrics.bounds.height()
        } else {
            val metrics = android.util.DisplayMetrics()
            @Suppress("DEPRECATION")
            windowManager.defaultDisplay.getRealMetrics(metrics)
            metrics.heightPixels
        }

    private fun showOverlay() {
        if (overlayAttached) return
        // A keyboard-relative pill needs the keyboard's bounds; without them it
        // would spawn mid-screen and visibly jump. The 100 ms IME check retries.
        if (followsKeyboard() && !isDictationActive() && imeTopPx() == null) return
        try {
            val view = VerenuOverlayView(this).apply {
                listener = this@VerenuAccessibilityService
                setDark(resolveDark())
                setCompact(isDocked())
            }
            val params = WindowManager.LayoutParams(
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                    WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
                    WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or
                    WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
                PixelFormat.TRANSLUCENT,
            )
            placeOverlay(params)
            windowManager.addView(view, params)
            overlay = view
            overlayParams = params
            overlayAttached = true
            setOverlayState(overlayState)
            view.animateIn()
        } catch (e: Exception) {
            Log.e(TAG, "cannot attach overlay", e)
        }
    }

    private fun hideOverlay() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            mainHandler.post { hideOverlay() }
            return
        }
        mainHandler.removeCallbacks(hideOverlayRunnable)
        overlayMoveAnimator?.cancel()
        val view = overlay ?: return
        overlay = null
        overlayParams = null
        overlayAttached = false
        view.animateOut {
            try {
                windowManager.removeView(view)
            } catch (e: Exception) {
                Log.w(TAG, "overlay already detached", e)
            }
        }
    }

    private fun setOverlayState(next: VerenuOverlayView.State) {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            mainHandler.post { setOverlayState(next) }
            return
        }
        overlayState = next
        if (next != VerenuOverlayView.State.ERROR) {
            overlayErrorMessage = ""
            errorAction = ErrorAction.RETRY_TRANSCRIPTION
        }
        if (next == VerenuOverlayView.State.ERROR || next == VerenuOverlayView.State.CANCELLED) {
            transientShownAtMs = System.currentTimeMillis()
        }
        // Whether the pill is shown (and docked) depends on this state.
        refreshOverlayVisibility()
        val view = overlay ?: return
        if (next == VerenuOverlayView.State.ERROR && overlayErrorMessage.isNotEmpty()) {
            view.setError(overlayErrorMessage)
        } else {
            view.updateState(next)
        }
    }

    /** Retain the short, already-sanitized bridge message across overlay churn. */
    private fun showOverlayError(
        message: String,
        action: ErrorAction = ErrorAction.RETRY_TRANSCRIPTION,
    ) {
        val safe = message.trim().ifEmpty { "Something went wrong" }
        overlayErrorMessage = safe.take(240)
        errorAction = action
        setOverlayState(VerenuOverlayView.State.ERROR)
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

    override fun onPillCancel() = requestCancel()

    private fun requestCancel() {
        if (cancelRequestInFlight) return
        val handler = requester
        if (handler == null) {
            showOverlayError("Could not cancel recording", ErrorAction.RETRY_CANCEL)
            return
        }
        cancelRequestInFlight = true
        handler.post {
            val resp = try {
                bridge.cancelRecording()
            } catch (e: Exception) {
                Log.w(TAG, "cancel failed", e)
                null
            }
            mainHandler.post {
                cancelRequestInFlight = false
                if (resp?.optBoolean("ok") == true) {
                    VerenuAnalytics.dictationCancelled(currentRunId)
                    stopDictationService()
                    setOverlayState(VerenuOverlayView.State.CANCELLED)
                } else {
                    // Keep the recording state and foreground notification if
                    // Rust did not acknowledge. A failed cancel must never
                    // strand an active microphone without its stop affordance.
                    showOverlayError("Couldn't cancel — check connection", ErrorAction.RETRY_CANCEL)
                }
            }
        }
    }

    override fun onPillRetry() {
        when (errorAction) {
            ErrorAction.RETRY_START -> {
                VerenuAnalytics.retryAttempted(currentRunId, "start")
                setOverlayState(VerenuOverlayView.State.IDLE)
                startDictation()
                return
            }
            ErrorAction.RETRY_STOP -> {
                VerenuAnalytics.retryAttempted(currentRunId, "stop")
                setOverlayState(VerenuOverlayView.State.RECORDING)
                stopDictation()
                return
            }
            ErrorAction.RETRY_CANCEL -> {
                VerenuAnalytics.retryAttempted(currentRunId, "cancel")
                requestCancel()
                return
            }
            ErrorAction.RETRY_INSERTION -> {
                VerenuAnalytics.retryAttempted(currentRunId, "insertion")
                lastInsertAttemptMs = 0L
                setOverlayState(VerenuOverlayView.State.INSERTING)
                return
            }
            ErrorAction.DISMISS -> {
                onPillDismiss()
                return
            }
            ErrorAction.RETRY_TRANSCRIPTION -> Unit
        }
        val handler = requester
        if (handler == null) {
            showOverlayError("Retry failed — check connection", ErrorAction.RETRY_TRANSCRIPTION)
            return
        }
        handler.post {
            VerenuAnalytics.retryAttempted(currentRunId, "transcription")
            val resp = try {
                bridge.retryTranscription()
            } catch (e: Exception) {
                Log.w(TAG, "retry failed", e)
                null
            }
            mainHandler.post {
                if (resp?.optBoolean("ok") == true) {
                    setOverlayState(VerenuOverlayView.State.TRANSCRIBING)
                } else {
                    showOverlayError("Retry failed — check connection", ErrorAction.RETRY_TRANSCRIPTION)
                }
            }
        }
    }

    override fun onPillDismiss() {
        if (!isDictationActive() &&
            errorAction != ErrorAction.RETRY_STOP &&
            errorAction != ErrorAction.RETRY_CANCEL
        ) {
            overlayErrorMessage = ""
            setOverlayState(VerenuOverlayView.State.IDLE)
            hideOverlay()
        }
    }

    private fun startDictation() {
        val pkg = foregroundPackage
        val editable = hasEditableFocus
        val setText = supportsSetText
        val handler = requester
        if (handler == null) {
            showOverlayError("Could not start recording", ErrorAction.RETRY_START)
            return
        }
        handler.post {
            // Make sure Rust is up first (cold-started service process).
            val backendUp = ensureBackendRunning(waitMs = BACKEND_START_WAIT_MS)
            if (!backendUp) {
                mainHandler.post {
                    showOverlayError("Verenu is still starting — try again", ErrorAction.RETRY_START)
                }
                return@post
            }
            // Promote to a microphone foreground service before opening the
            // Rust capture stream. Samsung's audio hardening can permanently
            // mark a background-created VOICE_RECOGNITION session as
            // silenced, even if foreground promotion happens immediately
            // afterward.
            if (!startDictationService()) {
                mainHandler.post {
                    showOverlayError("Could not start the microphone", ErrorAction.RETRY_START)
                }
                return@post
            }
            val resp = try {
                bridge.startRecording(pkg, editable, setText)
            } catch (e: Exception) {
                Log.w(TAG, "start recording failed", e)
                null
            }
            mainHandler.post {
                if (resp?.optBoolean("ok") == true) {
                    currentRunId = VerenuAnalytics.newRunId()
                    recordingStartedAtMs = System.currentTimeMillis()
                    lastPipelineStage = "recording"
                    VerenuAnalytics.dictationStarted(
                        currentRunId,
                        setText,
                        resp.optJSONObject("analyticsSettings"),
                    )
                    setOverlayState(VerenuOverlayView.State.RECORDING)
                } else {
                    // Do not leave a foreground notification behind when the
                    // backend rejected the recording request.
                    stopDictationService()
                    showOverlayError("Could not start recording", ErrorAction.RETRY_START)
                }
            }
        }
    }

    private fun stopDictation() {
        if (stopRequestInFlight) return
        val handler = requester
        if (handler == null) {
            showOverlayError("Couldn't stop recording — check connection", ErrorAction.RETRY_STOP)
            return
        }
        stopRequestInFlight = true
        handler.post {
            val resp = try {
                bridge.stopRecording()
            } catch (e: Exception) {
                Log.w(TAG, "stop recording failed", e)
                null
            }
            mainHandler.post {
                stopRequestInFlight = false
                if (resp?.optBoolean("ok") == true) {
                    VerenuAnalytics.recordingFinished(
                        currentRunId,
                        System.currentTimeMillis() - recordingStartedAtMs,
                    )
                    VerenuAnalytics.dictationStopped(currentRunId)
                    // The foreground service is needed only while Rust owns
                    // the microphone. Transcription and cleanup continue
                    // through the bridge after this acknowledgement.
                    stopDictationService()
                    setOverlayState(VerenuOverlayView.State.TRANSCRIBING)
                } else {
                    // Keep the service and recording alive until Rust accepts
                    // the stop, so the user still has a working stop/cancel
                    // path after a transient bridge failure.
                    showOverlayError(
                        "Couldn't stop recording — try again",
                        ErrorAction.RETRY_STOP,
                    )
                }
            }
        }
    }

    private fun retryDictation() = onPillRetry()

    private fun startDictationService(): Boolean {
        try {
            val intent = Intent(this, VerenuDictationService::class.java)
                .setAction(VerenuDictationService.ACTION_ACTIVE)
            if (Build.VERSION.SDK_INT >= 29) {
                startForegroundService(intent)
            } else {
                startService(intent)
            }
            return true
        } catch (e: Exception) {
            Log.w(TAG, "cannot start dictation service", e)
            return false
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
            val focus = findInputFocus()
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
     * Password fields are a hard stop: dictation never reads, writes, or
     * copies text for them. This is stricter than merely avoiding a read,
     * because a clipboard fallback would still expose dictated content.
     */
    private fun performInsertion(text: String): Pair<String, Boolean> {
        val node = focusedEditable()
        if (node == null) {
            return Pair("clipboard_fallback", copyToClipboard(text, tryPaste = false))
        }
        try {
            if (node.isPassword) return Pair("blocked_password", false)
            val actions = node.actionList.map { it.id }
            if (!actions.contains(AccessibilityNodeInfo.ACTION_SET_TEXT)) {
                return Pair("clipboard_fallback", copyToClipboard(text, tryPaste = true, node = node))
            }
            val ok = run {
                // An empty field reports its hint ("Search", "Message") as its
                // text. Treating that as content typed the hint into the field
                // in front of the dictation.
                val showingHint = Build.VERSION.SDK_INT >= 26 && node.isShowingHintText
                val before = if (showingHint) "" else node.text?.toString() ?: ""
                val selStart = if (showingHint) 0 else node.textSelectionStart.takeIf { it >= 0 } ?: before.length
                val selEnd = if (showingHint) 0 else node.textSelectionEnd.takeIf { it >= 0 } ?: selStart
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
                val pasted = node.performAction(AccessibilityNodeInfo.ACTION_PASTE)
                if (pasted) clipboard.clearPrimaryClip()
                pasted
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
        syncDeviceLockState()
        try {
            val snapshot = bridge.getState()
            val up = snapshot != null
            if (up && !backendWasUp) {
                // Rust restarted (or just started): its credential cache is
                // memory-only, so repopulate it from the Keystore.
                hydrateCredentials()
            }
            backendWasUp = up
            if (snapshot != null) {
                onBridgeState(snapshot)
            } else if (!keyboardVisible) {
                // Backend died or never started; relaunching while a field is
                // focused would steal focus from the user's app.
                ensureBackendRunning(waitMs = 0L)
            }
        } catch (e: Exception) {
            Log.w(TAG, "bridge poll failed", e)
        }
        // Transient pills auto-hide like the desktop (10s), and only when
        // nothing is in flight.
        if ((overlayState == VerenuOverlayView.State.ERROR ||
                overlayState == VerenuOverlayView.State.CANCELLED) &&
            !isDictationActive() &&
            errorAction != ErrorAction.RETRY_STOP &&
            errorAction != ErrorAction.RETRY_CANCEL &&
            System.currentTimeMillis() - transientShownAtMs > TRANSIENT_AUTO_HIDE_MS
        ) {
            hideOverlay()
        }
        schedulePoll(
            when {
                overlayState == VerenuOverlayView.State.RECORDING -> POLL_RECORDING_MS
                overlayAttached -> POLL_VISIBLE_MS
                else -> POLL_IDLE_MS
            },
        )
    }

    private fun onBridgeState(snapshot: BridgeStateSnapshot) {
        VerenuAnalytics.setEnabled(snapshot.analyticsEnabled)
        if (snapshot.pillPosition != pillPosition || snapshot.appearanceMode != appearanceMode) {
            pillPosition = snapshot.pillPosition
            appearanceMode = snapshot.appearanceMode
            mainHandler.post { applyOverlayPresentation() }
        }
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

        // The stop request can outlive the recording UI. Errors are mirrored
        // separately from lifecycle so a provider/auth/network failure cannot
        // leave the pill stuck on Transcribing forever.
        val bridgeError = snapshot.lastError
        if (bridgeError != null && isDictationActive() && bridgeError.ageMs < 120_000L) {
            VerenuAnalytics.pipelineFailed(currentRunId, bridgeError.message, snapshot.pillStage)
            stopDictationService()
            mainHandler.post {
                showOverlayError(bridgeError.message)
            }
            return
        }

        // Pill stage refinement while a dictation is in flight.
        if (isDictationActive()) {
            if (snapshot.pillStage != lastPipelineStage) {
                lastPipelineStage = snapshot.pillStage
                VerenuAnalytics.pipelineStageStarted(currentRunId, snapshot.pillStage)
            }
            when (snapshot.pillStage) {
                "cleaning" -> if (overlayState == VerenuOverlayView.State.TRANSCRIBING) {
                    setOverlayState(VerenuOverlayView.State.CLEANING)
                }
                "pasting" -> setOverlayState(VerenuOverlayView.State.INSERTING)
            }
            val level = snapshot.audioLevel
            val envelope = snapshot.audioEnvelope
            mainHandler.post {
                if (envelope.isNotEmpty()) overlay?.setEnvelope(envelope) else overlay?.setAudioLevel(level)
            }
            if (!snapshot.dictationActive && overlayState == VerenuOverlayView.State.RECORDING) {
                // Backend left recording without our stop (gate rejection,
                // error): fall back to transcribing/error via lifecycle.
                setOverlayState(VerenuOverlayView.State.TRANSCRIBING)
            }

            // A successful pipeline with no pending insertion has completed.
            // Normally the insertion ack handles this; this fallback covers
            // empty/copy-only completions and keeps the native pill in sync
            // when the target app disappears during processing.
            if (snapshot.lifecycle == "idle" &&
                !snapshot.dictationActive &&
                snapshot.pendingInsertion == null &&
                overlayState != VerenuOverlayView.State.RECORDING
            ) {
                stopDictationService()
                setOverlayState(VerenuOverlayView.State.IDLE)
                if (!shouldShowOverlay()) hideOverlay()
            }
        }

        // Insertion handoff. The target is the field that is focused NOW, not
        // a package remembered from window events: on Samsung, one-hand mode,
        // the edge panel and notification shade all emit events from other
        // packages, which used to leave a finished dictation waiting for a
        // "foreground" match that never came (the pill sat on "Pasting…" for a
        // minute). If no field is available the text is copied instead, and the
        // outbox is always cleared, so there is no endless retry loop.
        val current = snapshot.pendingInsertion
        if (current == null) {
            lastInsertedSeq = -1L
            lastInsertedPackage = ""
            return
        }
        if (current.seq != lastSeenPendingSeq) {
            lastSeenPendingSeq = current.seq
            insertionStartedAtMs = System.currentTimeMillis() - current.ageMs
            lastInsertAttemptMs = 0L
        }
        if (current.seq == lastInsertedSeq) {
            // The text is already in the field. Only retry the idempotent ack;
            // never perform ACTION_SET_TEXT twice for the same sequence.
            val now = System.currentTimeMillis()
            if (now - lastInsertionAckAttemptMs > INSERT_RETRY_MS) {
                lastInsertionAckAttemptMs = now
                val ack = bridge.ackInsertion(
                    current.seq,
                    true,
                    lastInsertedStrategy,
                    null,
                    lastInsertedPackage,
                )
                if (ack?.optBoolean("ok") == true) {
                    lastInsertedSeq = -1L
                    lastInsertedPackage = ""
                }
            }
            return
        }

        val now = System.currentTimeMillis()
        val waitedMs = current.ageMs
        val canAttempt = now - lastInsertAttemptMs > INSERT_ATTEMPT_MS
        if (!canAttempt) return

        if (insertionTargetAvailable(snapshot.targetPackage)) {
            lastInsertAttemptMs = now
            VerenuAnalytics.insertionAttempted(currentRunId)
            val (strategy, ok) = performInsertion(current.text)
            val blockedPassword = strategy == "blocked_password"
            // A failed ACTION_PASTE still leaves the text in the clipboard;
            // make the manual-copy result explicit before acknowledging so a
            // successful copy clears the outbox and is not repeated every
            // poll. Password fields never enter this path.
            val manualCopy = !ok && !blockedPassword &&
                copyToClipboard(current.text, tryPaste = false)
            val ack = bridge.ackInsertion(
                current.seq,
                ok,
                strategy,
                if (blockedPassword) "password_field" else null,
                foregroundPackage,
                discard = blockedPassword || manualCopy,
            )
            if (ok || manualCopy) {
                VerenuAnalytics.dictationInserted(
                    currentRunId,
                    if (ok) strategy else "clipboard_fallback",
                    current.text.trim().split(Regex("\\s+")).count { it.isNotEmpty() },
                )
                if (!ok && manualCopy) VerenuAnalytics.fallbackUsed(currentRunId, "manual_copy")
                // Consider the local edit successful even if the ack response
                // is lost. The next poll retries only this ack, avoiding a
                // second edit while still allowing Rust to clear its outbox.
                lastInsertedSeq = current.seq
                lastInsertedStrategy = if (ok) strategy else "clipboard_fallback"
                lastInsertedPackage = foregroundPackage
                lastInsertionAckAttemptMs = System.currentTimeMillis()
                stopDictationService()
                if (ok) {
                    setOverlayState(VerenuOverlayView.State.IDLE)
                } else {
                    mainHandler.post {
                        showOverlayError("Couldn't insert — copied instead", ErrorAction.DISMISS)
                    }
                }
            } else if (ack?.optBoolean("ok") == true || !ok) {
                VerenuAnalytics.pipelineFailed(currentRunId, "insertion failed", "inserting")
                stopDictationService()
                if (blockedPassword) {
                    mainHandler.post {
                        showOverlayError(
                            "Dictation is disabled in password fields",
                            ErrorAction.DISMISS,
                        )
                    }
                } else {
                    mainHandler.post {
                        showOverlayError(
                            "Couldn't insert — refocus the field and retry",
                            ErrorAction.RETRY_INSERTION,
                        )
                    }
                }
            }
            return
        }

        // No usable field right now (keyboard closed, focus moved). Give the
        // user a few seconds to come back to one, then stop waiting.
        if (waitedMs >= INSERT_WAIT_FOR_FIELD_MS) {
            lastInsertAttemptMs = now
            val copied = copyToClipboard(current.text, tryPaste = false)
            bridge.ackInsertion(
                current.seq,
                false,
                "clipboard_fallback",
                if (waitedMs >= INSERT_GIVE_UP_MS) "expired" else "no_target",
                foregroundPackage,
                discard = true,
            )
            lastInsertedSeq = current.seq
            lastInsertedStrategy = "clipboard_fallback"
            lastInsertedPackage = foregroundPackage
            stopDictationService()
            VerenuAnalytics.pipelineFailed(currentRunId, "no insertion target", "inserting")
            mainHandler.post {
                showOverlayError(
                    if (copied) "No text field — copied instead" else "No text field to paste into",
                    ErrorAction.DISMISS,
                )
            }
        }
    }

    /**
     * Whether a focused editable field exists that this dictation may fill: it
     * must belong to the app the dictation started in (when we know it), or at
     * least to whatever app is in front.
     */
    private fun insertionTargetAvailable(targetPackage: String): Boolean {
        val node = focusedEditable() ?: return false
        val owner = try {
            node.packageName?.toString().orEmpty()
        } finally {
            node.recycle()
        }
        return targetPackage.isEmpty() ||
            owner.isEmpty() ||
            owner == targetPackage ||
            owner == foregroundPackage
    }
}
