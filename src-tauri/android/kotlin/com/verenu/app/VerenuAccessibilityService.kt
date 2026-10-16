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
import android.os.SystemClock
import android.provider.Settings
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
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
        private const val IME_SETTLE_MS = 40L
        private const val CONTEXT_CHARS = 200
        private const val SNOOZE_MS = 15L * 60_000L
        private const val PLACEMENT_SYNC_MAX_AGE_MS = 750L
        private const val PLACEMENT_PREFS = "verenu_pill_placement"
        private const val PREF_POSITION = "position"
        private const val PREF_DOCK_POSITION = "dock_position"
        private const val PREF_COVER_MIC = "cover_keyboard_mic"
        private val BROWSER_URL_BAR_IDS = mapOf(
            "com.android.chrome" to listOf("url_bar"),
            "com.chrome.beta" to listOf("url_bar"),
            "com.chrome.dev" to listOf("url_bar"),
            "com.chrome.canary" to listOf("url_bar"),
            "com.brave.browser" to listOf("url_bar"),
            "com.microsoft.emmx" to listOf("url_bar"),
            "com.vivaldi.browser" to listOf("url_bar"),
            "com.kiwibrowser.browser" to listOf("url_bar"),
            "com.opera.browser" to listOf("url_bar", "url_field"),
            "com.sec.android.app.sbrowser" to listOf("location_bar_edit_text"),
            "org.mozilla.firefox" to listOf("mozac_browser_toolbar_url_view"),
            "org.mozilla.firefox_beta" to listOf("mozac_browser_toolbar_url_view"),
            "org.mozilla.fenix" to listOf("mozac_browser_toolbar_url_view"),
            "com.duckduckgo.mobile.android" to listOf("omnibarTextInput"),
        )

        internal fun hostFromAddressText(raw: String): String {
            val text = raw.trim().lowercase()
            if (text.isEmpty() || text.any { it.isWhitespace() }) return ""
            val host = text.substringAfter("://").substringBefore('/').substringBefore('?')
                .substringBefore('#').substringAfterLast('@').substringBefore(':')
                .removePrefix("www.")
            return if (host.contains('.')) host else ""
        }

        private val MIC_LABEL = Regex("voice|dictat|microphone|speech|\\bmic\\b", RegexOption.IGNORE_CASE)
        private val MIC_EXCLUDE = Regex("permission|settings|language|\\bsend\\b", RegexOption.IGNORE_CASE)
        const val TAG = "VerenuA11y"
        const val POLL_VISIBLE_MS = 250L
        const val POLL_IDLE_MS = 2000L
        const val POLL_MUTING_MS = 100L
        const val INSERT_RETRY_MS = 2000L
        const val PENDING_MAX_AGE_MS = 60_000L
        const val TRANSIENT_AUTO_HIDE_MS = 10_000L
        const val BACKEND_LAUNCH_COOLDOWN_MS = 20_000L
        const val BACKEND_START_WAIT_MS = 12_000L
        const val HIDE_DEBOUNCE_MS = 450L
        const val MIC_MISS_RETRY_MS = 120L
        const val BROWSER_SITE_TTL_MS = 10L * 60_000L
        const val BROWSER_SCAN_INTERVAL_MS = 1_500L
        const val POLL_RECORDING_MS = 60L
        const val INSERT_ATTEMPT_MS = 350L
        const val INSERT_WAIT_FOR_FIELD_MS = 6_000L
        const val INSERT_GIVE_UP_MS = 12_000L

        internal fun nextPollDelayMs(
            recording: Boolean,
            audioMuteActive: Boolean,
            overlayAttached: Boolean,
            syncMutingEnabled: Boolean,
        ): Long = when {
            recording -> POLL_RECORDING_MS
            audioMuteActive || overlayAttached -> POLL_VISIBLE_MS
            syncMutingEnabled -> POLL_MUTING_MS
            else -> POLL_IDLE_MS
        }

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
    @Volatile private var audioMuteActive = false
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
    private var gliding = false
    @Volatile private var pillPosition = VerenuPillPlacement.DEFAULT_POSITION
    @Volatile private var pillDockPosition = VerenuPillPlacement.DEFAULT_DOCK_POSITION
    @Volatile private var coverKeyboardMic = true
    @Volatile private var hidePillOffline = true
    private val pendingPlacementSaves = java.util.concurrent.atomic.AtomicInteger(0)
    @Volatile private var lastPlacementSyncMs = 0L
    private var coverBounds: android.graphics.Rect? = null
    private var lastCoverState = ""
    private var coverSizePx = 0

    private var snoozedUntilMs = 0L
    private var dragging = false
    private var snoozeTarget: View? = null
    private val snoozeTargetRect = android.graphics.Rect()
    @Volatile private var online = true
    private var netCallback: android.net.ConnectivityManager.NetworkCallback? = null
    private var micCache: android.graphics.Rect? = null
    private var micCacheKey: android.graphics.Rect? = null
    private var micCacheAtMs = 0L
    @Volatile private var appearanceMode = "system"
    @Volatile private var backendWasUp = false

    override fun onServiceConnected() {
        bridge = VerenuBridge(this)
        windowManager = getSystemService(WINDOW_SERVICE) as WindowManager
        deviceLocked = isDeviceLocked()
        loadPlacementPrefs()
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
        startConnectivityWatch()
        Log.i(TAG, "connected")
    }

    override fun onUnbind(intent: Intent?): Boolean {
        stopConnectivityWatch()
        mainHandler.removeCallbacks(imeVisibilityCheck)
        poller?.removeCallbacksAndMessages(null)
        requester?.removeCallbacksAndMessages(null)
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

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        if (event == null) return
        val pkg = event.packageName?.toString() ?: ""
        val imePackage = defaultImePackage()
        if (Log.isLoggable(TAG, Log.DEBUG)) {
            Log.d(TAG, "event type=${AccessibilityEvent.eventTypeToString(event.eventType)} pkg=$pkg ime=$imePackage")
        }
        foregroundPackage = VerenuFocusPackages.eventTarget(
            foregroundPackage, pkg, event.eventType == AccessibilityEvent.TYPE_VIEW_FOCUSED,
            packageName, imePackage,
        )
        if (event.eventType == AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED) {
            refreshForegroundPackage(imePackage)
        }
        if (pkg == foregroundPackage && BROWSER_URL_BAR_IDS.containsKey(pkg) &&
            (event.eventType == AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED ||
                event.eventType == AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED ||
                event.eventType == AccessibilityEvent.TYPE_VIEW_FOCUSED)
        ) {
            rememberBrowserDomain(pkg)
        }
        when (event.eventType) {
            AccessibilityEvent.TYPE_VIEW_FOCUSED -> {
                val node = event.source
                try {
                    val editable = node?.isEditable == true && node.isPassword.not()
                    if (!editable && !VerenuFocusPackages.isInsertionField(pkg, imePackage)) return
                    hasEditableFocus = editable
                    if (editable) {
                        val owner = node?.packageName?.toString().orEmpty()
                        if (VerenuFocusPackages.isAppWindow(owner, packageName, imePackage)) {
                            foregroundPackage = owner
                        }
                    }
                    supportsSetText = editable &&
                        node?.actionList?.any { it.id == AccessibilityNodeInfo.ACTION_SET_TEXT } == true
                    if (editable) onKeyboardChanged(true)
                    refreshOverlayVisibility()
                } finally {
                    node?.recycle()
                }
            }
            AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED,
            AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED,
            -> {
            }
            AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED -> {
                if (pkg.isNotEmpty() && pkg == imePackage) {
                    onKeyboardChanged(true)
                } else if (pkg != packageName) {
                    val node = findInputFocus()
                    val editable = node?.isEditable == true && node.isPassword.not()
                    node?.recycle()
                    if (editable || !keyboardVisible) hasEditableFocus = editable
                }
            }
        }
        scheduleKeyboardVisibilityCheck()
    }

    private fun scheduleKeyboardVisibilityCheck() {
        mainHandler.removeCallbacks(imeVisibilityCheck)
        mainHandler.postDelayed(imeVisibilityCheck, if (keyboardVisible && !overlayAttached) 16L else 100L)
    }

    private fun defaultImePackage(): String? = Settings.Secure.getString(
        contentResolver,
        Settings.Secure.DEFAULT_INPUT_METHOD,
    )?.substringBefore('/')

    private fun refreshForegroundPackage(imePackage: String?) {
        val interactiveWindows = windows
        try {
            for (window in interactiveWindows) {
                if (window.type != AccessibilityWindowInfo.TYPE_APPLICATION || !window.isFocused) continue
                val root = window.root ?: continue
                try {
                    val owner = root.packageName?.toString().orEmpty()
                    if (VerenuFocusPackages.isAppWindow(owner, packageName, imePackage)) {
                        foregroundPackage = owner
                    }
                } finally {
                    root.recycle()
                }
            }
        } finally {
            interactiveWindows.forEach { it.recycle() }
        }
    }

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

    private fun findInputFocus(
        budget: VerenuInsertionBudget = VerenuInsertionBudget(SystemClock::elapsedRealtime, 350L),
    ): AccessibilityNodeInfo? {
        val direct = findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
        if (direct != null && direct.isEditable) return direct
        direct?.recycle()
        if (budget.expired) return null
        val root = rootInActiveWindow ?: return null
        var nodesLeft = 64
        fun walk(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
            if (budget.expired || nodesLeft-- <= 0) return null
            if (node.isFocused && node.isEditable) return AccessibilityNodeInfo.obtain(node)
            for (i in 0 until node.childCount) {
                if (budget.expired || nodesLeft <= 0) return null
                val child = node.getChild(i) ?: continue
                val found = try { walk(child) } finally { child.recycle() }
                if (found != null) return found
            }
            return null
        }
        return try { walk(root) } finally { root.recycle() }
    }

    private fun refreshKeyboardVisibilityFromWindows() {
        val imeVisible = try {
            windows.any {
                it.type == android.view.accessibility.AccessibilityWindowInfo.TYPE_INPUT_METHOD
            }
        } catch (e: Exception) {
            Log.w(TAG, "IME window state unavailable", e)
            return
        }
        if (imeVisible && (!keyboardVisible || !hasEditableFocus)) {
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
            if (keyboardVisible) onKeyboardChanged(false) else refreshOverlayVisibility()
        }
        if (keyboardVisible) {
            if (overlayAttached) applyOverlayPresentation() else refreshOverlayVisibility()
            mainHandler.postDelayed(imeVisibilityCheck, if (overlayAttached) 60L else 30L)
        }
    }

    private fun onKeyboardChanged(visible: Boolean) {
        if (Log.isLoggable(TAG, Log.DEBUG)) Log.d(TAG, "keyboardChanged visible=$visible editable=$hasEditableFocus")
        if (keyboardVisible == visible && visible) return
        keyboardVisible = visible
        if (!visible) hasEditableFocus = false
        runOnBridge { bridge.postFocus(visible, hasEditableFocus) }
        refreshOverlayVisibility()
        if (visible) refreshPlacementSettings()
    }

    private fun applyPlacementSettings(snapshot: BridgeStateSnapshot) {
        lastPlacementSyncMs = SystemClock.elapsedRealtime()
        val placementChanged = pendingPlacementSaves.get() == 0 &&
            (snapshot.pillPosition != pillPosition ||
                snapshot.pillDockPosition != pillDockPosition ||
                snapshot.coverKeyboardMic != coverKeyboardMic)
        if (!placementChanged &&
            snapshot.appearanceMode == appearanceMode &&
            snapshot.hidePillOffline == hidePillOffline
        ) {
            return
        }
        if (placementChanged) {
            coverKeyboardMic = snapshot.coverKeyboardMic
            pillPosition = snapshot.pillPosition
            pillDockPosition = snapshot.pillDockPosition
            savePlacementPrefs()
        }
        hidePillOffline = snapshot.hidePillOffline
        appearanceMode = snapshot.appearanceMode
        mainHandler.post {
            refreshOverlayVisibility()
            applyOverlayPresentation()
        }
    }

    private fun refreshPlacementSettings() {
        if (SystemClock.elapsedRealtime() - lastPlacementSyncMs < PLACEMENT_SYNC_MAX_AGE_MS) return
        runOnBridge { bridge.getState()?.let { applyPlacementSettings(it) } }
    }

    private fun placementPrefs() = getSharedPreferences(PLACEMENT_PREFS, Context.MODE_PRIVATE)

    private fun loadPlacementPrefs() {
        try {
            val prefs = placementPrefs()
            pillPosition = VerenuPillPlacement.sanitizePosition(prefs.getString(PREF_POSITION, null))
            pillDockPosition = VerenuPillPlacement.sanitizeDockPosition(prefs.getString(PREF_DOCK_POSITION, null))
            coverKeyboardMic = prefs.getBoolean(PREF_COVER_MIC, true)
        } catch (e: Exception) {
            Log.w(TAG, "placement prefs unavailable", e)
        }
    }

    private fun savePlacementPrefs() {
        try {
            placementPrefs().edit()
                .putString(PREF_POSITION, pillPosition)
                .putString(PREF_DOCK_POSITION, pillDockPosition)
                .putBoolean(PREF_COVER_MIC, coverKeyboardMic)
                .apply()
        } catch (e: Exception) {
            Log.w(TAG, "placement prefs not saved", e)
        }
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

    private fun shouldShowOverlay(): Boolean =
        (keyboardVisible && hasEditableFocus && !suppressedForNow()) ||
            isDictationActive() ||
            transientNoticeVisible()

    private fun suppressedForNow(): Boolean =
        SystemClock.elapsedRealtime() < snoozedUntilMs || (hidePillOffline && !online)

    private fun startConnectivityWatch() {
        val cm = getSystemService(android.net.ConnectivityManager::class.java) ?: return
        online = hasInternet(cm)
        val callback = object : android.net.ConnectivityManager.NetworkCallback() {
            private fun update() {
                val now = hasInternet(cm)
                if (now == online) return
                online = now
                mainHandler.post { refreshOverlayVisibility() }
            }
            override fun onAvailable(network: android.net.Network) = update()
            override fun onLost(network: android.net.Network) = update()
            override fun onCapabilitiesChanged(
                network: android.net.Network,
                capabilities: android.net.NetworkCapabilities,
            ) = update()
        }
        try {
            cm.registerDefaultNetworkCallback(callback)
            netCallback = callback
        } catch (e: Exception) {
            Log.w(TAG, "connectivity watch unavailable", e)
        }
    }

    private fun stopConnectivityWatch() {
        val callback = netCallback ?: return
        netCallback = null
        try {
            getSystemService(android.net.ConnectivityManager::class.java)?.unregisterNetworkCallback(callback)
        } catch (e: Exception) {
            Log.w(TAG, "connectivity watch stop failed", e)
        }
    }

    private fun hasInternet(cm: android.net.ConnectivityManager): Boolean {
        val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return false
        return caps.hasCapability(android.net.NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
            (caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI) ||
                caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_CELLULAR) ||
                caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_ETHERNET))
    }

    private fun transientNoticeVisible(): Boolean =
        overlayState == VerenuOverlayView.State.ERROR &&
            System.currentTimeMillis() - transientShownAtMs < TRANSIENT_AUTO_HIDE_MS

    private fun refreshOverlayVisibility() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            mainHandler.post { refreshOverlayVisibility() }
            return
        }
        if (shouldShowOverlay()) {
            mainHandler.removeCallbacks(hideOverlayRunnable)
            hideScheduled = false
            showOverlay()
            applyOverlayPresentation()
        } else if (overlayAttached) {
            if (!hideScheduled) {
                hideScheduled = true
                mainHandler.postDelayed(hideOverlayRunnable, HIDE_DEBOUNCE_MS)
            }
        }
    }

    private var hideScheduled = false

    private val hideOverlayRunnable = Runnable {
        hideScheduled = false
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

    private fun cutoutRect(): android.graphics.Rect? {
        if (Build.VERSION.SDK_INT < 28) return null
        val cutout = try {
            if (Build.VERSION.SDK_INT >= 30) {
                windowManager.currentWindowMetrics.windowInsets.displayCutout
            } else {
                @Suppress("DEPRECATION")
                windowManager.defaultDisplay?.cutout
            }
        } catch (e: Exception) {
            null
        } ?: return null
        return cutout.boundingRects.firstOrNull { !it.isEmpty }
    }

    private fun statusBarHeightPx(): Int {
        val resourceId = resources.getIdentifier("status_bar_height", "dimen", "android")
        return if (resourceId != 0) resources.getDimensionPixelSize(resourceId) else 0
    }

    private fun followsKeyboard() = pillPosition.startsWith("keyboard-")

    private fun isDocked(): Boolean =
        isDictationActive() && !(keyboardVisible && hasEditableFocus && imeTopPx() != null)

    private fun placeOverlay(params: WindowManager.LayoutParams) {
        val density = resources.displayMetrics.density
        val margin = (12 * density).toInt()
        val docked = isDocked()
        val imeTop = if (keyboardVisible) imeTopPx() else null
        val screenWidth = realScreenWidthPx()
        val imeBounds = if (keyboardVisible) imeBoundsPx() else null

        val cover = coverBounds
        when {
            cover != null && !docked && imeTop != null -> {
                val pad = (4 * density).toInt()
                val size = coverSizePx
                val imeRight = (imeBounds?.right ?: screenWidth).coerceAtMost(screenWidth)
                val imeTopEdge = imeBounds?.top ?: 0
                val right = minOf(cover.centerX() + size / 2, imeRight - (2 * density).toInt())
                val viewHeight = overlay?.height?.takeIf { it > 0 } ?: (size + 2 * pad)
                params.gravity = Gravity.TOP or Gravity.RIGHT
                params.x = (screenWidth - right - pad).coerceAtLeast(0)
                params.y = (cover.centerY() - viewHeight / 2).coerceAtLeast(imeTopEdge - pad).coerceAtLeast(0)
            }
            docked && followsKeyboard() -> placeOnScreen(params, pillDockPosition)
            !followsKeyboard() -> placeOnScreen(params, pillPosition)
            imeTop != null -> {
                val above = (realScreenHeightPx() - imeTop + (8 * density).toInt()).coerceAtLeast(0)
                params.y = above
                val left = (imeBounds?.left ?: 0).coerceIn(0, screenWidth)
                val right = (imeBounds?.right ?: screenWidth).coerceIn(left, screenWidth)
                when (pillPosition) {
                    "keyboard-left" -> {
                        params.gravity = Gravity.BOTTOM or Gravity.LEFT
                        params.x = left + margin
                    }
                    "keyboard-right" -> {
                        params.gravity = Gravity.BOTTOM or Gravity.RIGHT
                        params.x = screenWidth - right + margin
                    }
                    else -> {
                        params.gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
                        params.x = (left + right) / 2 - screenWidth / 2
                    }
                }
            }
            else -> placeOnScreen(params, pillDockPosition)
        }
    }

    private fun placeOnScreen(params: WindowManager.LayoutParams, position: String) {
        val density = resources.displayMetrics.density
        val margin = (12 * density).toInt()
        val top = statusBarHeightPx() + (8 * density).toInt()
        val bottom = navigationBarHeightPx() + margin
        when (position) {
            "punch-hole" -> {
                val cutout = cutoutRect()
                if (cutout != null) {
                    params.gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL
                    params.x = cutout.centerX() - realScreenWidthPx() / 2
                    params.y = cutout.bottom + (4 * density).toInt()
                } else {
                    placeOnScreen(params, "screen-top")
                }
            }
            "screen-top-left" -> edgePlacement(params, Gravity.TOP or Gravity.LEFT, margin, top)
            "screen-top" -> edgePlacement(params, Gravity.TOP or Gravity.CENTER_HORIZONTAL, 0, top)
            "screen-top-right" -> edgePlacement(params, Gravity.TOP or Gravity.RIGHT, margin, top)
            "screen-left" -> edgePlacement(params, Gravity.CENTER_VERTICAL or Gravity.LEFT, margin, 0)
            "screen-middle" -> edgePlacement(params, Gravity.CENTER, 0, 0)
            "screen-right" -> edgePlacement(params, Gravity.CENTER_VERTICAL or Gravity.RIGHT, margin, 0)
            "screen-bottom-left" -> edgePlacement(params, Gravity.BOTTOM or Gravity.LEFT, margin, bottom)
            "screen-bottom-right" -> edgePlacement(params, Gravity.BOTTOM or Gravity.RIGHT, margin, bottom)
            else -> edgePlacement(params, Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL, 0, bottom)
        }
    }

    private fun edgePlacement(params: WindowManager.LayoutParams, gravity: Int, x: Int, y: Int) {
        params.gravity = gravity
        params.x = x
        params.y = y
    }

    private fun navigationBarHeightPx(): Int = try {
        if (Build.VERSION.SDK_INT >= 30) {
            windowManager.currentWindowMetrics.windowInsets
                .getInsetsIgnoringVisibility(android.view.WindowInsets.Type.navigationBars()).bottom
        } else {
            val id = resources.getIdentifier("navigation_bar_height", "dimen", "android")
            if (id != 0) resources.getDimensionPixelSize(id) else 0
        }
    } catch (e: Exception) {
        0
    }

    private fun applyOverlayPresentation() {
        val view = overlay ?: return
        if (keyboardVisible && imeBoundsSettled()) imeBoundsPx()?.let { rememberImeHeight(it) }
        view.setDark(resolveDark())
        view.setCompact(isDocked())
        refreshCover(view)
        repositionOverlay()
    }

    private fun refreshCover(view: VerenuOverlayView, preferPrediction: Boolean = false) {
        val rect = if (coverKeyboardMic && followsKeyboard() && !isDocked() && keyboardVisible) {
            if (preferPrediction) predictedMicBounds() ?: keyboardMicBounds() else keyboardMicBounds()
        } else {
            null
        }
        if (Log.isLoggable(TAG, Log.DEBUG)) {
            val state = "$rect/$coverKeyboardMic/${followsKeyboard()}/${isDocked()}/$keyboardVisible"
            if (state != lastCoverState) {
                lastCoverState = state
                Log.d(TAG, "cover state rect/enabled/follows/docked/kb=$state")
            }
        }
        coverBounds = rect
        val density = resources.displayMetrics.density
        val size = rect?.let {
            minOf(minOf(it.width(), it.height()) + (6 * density).toInt(), (56 * density).toInt())
        } ?: 0
        coverSizePx = size
        view.setCoverSize(size)
    }

    private fun keyboardMicBounds(): android.graphics.Rect? {
        val ime = imeBoundsPx() ?: return null
        if (imeIsSliding()) return null
        val now = SystemClock.elapsedRealtime()
        val ttl = if (micCache == null) MIC_MISS_RETRY_MS else 600L
        val fresh = micCacheKey == ime && now - micCacheAtMs < ttl
        if (fresh) return micCache
        micCacheKey = android.graphics.Rect(ime)
        micCacheAtMs = now
        micCache = try {
            val visibleWindows = windows
            try {
                val imeWindow = visibleWindows.firstOrNull {
                    it.type == android.view.accessibility.AccessibilityWindowInfo.TYPE_INPUT_METHOD
                } ?: return null
                val root = imeWindow.root ?: return null
                try {
                    findMicNode(root, ime.width(), 0)
                } finally {
                    root.recycle()
                }
            } finally {
                visibleWindows.forEach { it.recycle() }
            }
        } catch (e: Exception) {
            Log.w(TAG, "keyboard mic lookup failed", e)
            null
        }
        if (Log.isLoggable(TAG, Log.DEBUG)) Log.d(TAG, "keyboard mic lookup ime=$ime found=$micCache")
        micCache?.let { rememberMicPlacement(ime, it) }
        return micCache
    }

    private val placementPrefs by lazy { getSharedPreferences("verenu_overlay_placement", MODE_PRIVATE) }

    private fun placementKey(ime: android.graphics.Rect) =
        "mic:${defaultImePackage().orEmpty()}:${ime.width()}"

    private fun rememberMicPlacement(ime: android.graphics.Rect, mic: android.graphics.Rect) {
        if (!android.graphics.Rect(ime).contains(mic)) return
        val value = "${mic.left - ime.left},${ime.bottom - mic.top},${mic.width()},${mic.height()}"
        if (placementPrefs.getString(placementKey(ime), null) != value) {
            placementPrefs.edit().putString(placementKey(ime), value).apply()
        }
    }

    private var lastRememberedImeHeight = -1

    private fun imeHeightKey(ime: android.graphics.Rect) =
        "h:${defaultImePackage().orEmpty()}:${ime.width()}"

    private fun rememberImeHeight(ime: android.graphics.Rect) {
        if (ime.height() == lastRememberedImeHeight) return
        lastRememberedImeHeight = ime.height()
        placementPrefs.edit().putInt(imeHeightKey(ime), ime.height()).apply()
    }

    private fun imeLayoutKnown(): Boolean {
        val ime = imeBoundsPx() ?: return false
        return placementPrefs.getInt(imeHeightKey(ime), -1) == ime.height()
    }

    private fun predictedMicBounds(): android.graphics.Rect? {
        val ime = imeBoundsPx() ?: return null
        val parts = placementPrefs.getString(placementKey(ime), null)?.split(',')?.mapNotNull { it.toIntOrNull() }
        if (parts == null || parts.size != 4) return null
        val (dx, fromBottom, w, h) = parts
        val rect = android.graphics.Rect(ime.left + dx, ime.bottom - fromBottom, ime.left + dx + w, ime.bottom - fromBottom + h)
        return if (ime.contains(rect)) rect else null
    }

    private fun findMicNode(node: AccessibilityNodeInfo, imeWidth: Int, depth: Int): android.graphics.Rect? {
        if (depth > 14) return null
        val label = "${node.contentDescription ?: ""} ${node.text ?: ""} ${node.viewIdResourceName ?: ""}"
        if (node.isVisibleToUser && MIC_LABEL.containsMatchIn(label) && !MIC_EXCLUDE.containsMatchIn(label)) {
            val rect = android.graphics.Rect()
            node.getBoundsInScreen(rect)
            if (rect.width() > 0 && rect.height() > 0 && rect.width() < imeWidth / 3) return rect
        }
        for (i in 0 until node.childCount) {
            val child = node.getChild(i) ?: continue
            val found = try {
                findMicNode(child, imeWidth, depth + 1)
            } finally {
                child.recycle()
            }
            if (found != null) return found
        }
        return null
    }

    private fun resolveDark(): Boolean = when (appearanceMode) {
        "dark" -> true
        "light" -> false
        else -> (resources.configuration.uiMode and android.content.res.Configuration.UI_MODE_NIGHT_MASK) ==
            android.content.res.Configuration.UI_MODE_NIGHT_YES
    }

    private fun repositionOverlay() {
        if (dragging || gliding) return
        val view = overlay ?: return
        val params = overlayParams ?: return
        val fromY = params.y
        val fromX = params.x
        val fromGravity = params.gravity
        placeOverlay(params)
        val toY = params.y
        val toX = params.x
        if (fromGravity != params.gravity) {
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

    /**
     * The keyboard window's bounds, as they will be once it has finished sliding
     * in. While it slides its window is reported hanging off the bottom of the
     * screen at full height; shifting it up by the overhang gives the final
     * position straight away, so the pill does not have to wait out the animation.
     */
    private fun imeBoundsPx(): android.graphics.Rect? = try {
        val rect = android.graphics.Rect()
        windows
            .firstOrNull { it.type == android.view.accessibility.AccessibilityWindowInfo.TYPE_INPUT_METHOD }
            ?.let {
                it.getBoundsInScreen(rect)
                val overhang = slideOverhang(rect)
                if (overhang > 0) rect.offset(0, -overhang)
                if (rect.height() > 0) rect else null
            }
    } catch (e: Exception) {
        null
    }

    private fun slideOverhang(raw: android.graphics.Rect): Int {
        val overhang = raw.bottom - realScreenHeightPx()
        return if (overhang > 2 && overhang < raw.height()) overhang else 0
    }

    private fun imeIsSliding(): Boolean = try {
        val rect = android.graphics.Rect()
        windows
            .firstOrNull { it.type == android.view.accessibility.AccessibilityWindowInfo.TYPE_INPUT_METHOD }
            ?.let { it.getBoundsInScreen(rect); slideOverhang(rect) > 0 } ?: false
    } catch (e: Exception) {
        false
    }

    private fun imeTopPx(): Int? = imeBoundsPx()?.top

    private var settleBounds: android.graphics.Rect? = null
    private var settleSinceMs = 0L

    private fun imeBoundsSettled(): Boolean {
        val bounds = imeBoundsPx()
        if (bounds == null) {
            settleBounds = null
            return false
        }
        if (bounds.bottom > realScreenHeightPx() + 2) {
            settleBounds = null
            return false
        }
        val now = SystemClock.elapsedRealtime()
        if (settleBounds != bounds) {
            settleBounds = android.graphics.Rect(bounds)
            settleSinceMs = now
            return false
        }
        return now - settleSinceMs >= IME_SETTLE_MS
    }

    private fun realScreenWidthPx(): Int =
        if (Build.VERSION.SDK_INT >= 30) {
            windowManager.currentWindowMetrics.bounds.width()
        } else {
            val metrics = android.util.DisplayMetrics()
            @Suppress("DEPRECATION")
            windowManager.defaultDisplay.getRealMetrics(metrics)
            metrics.widthPixels
        }

    override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
        super.onConfigurationChanged(newConfig)
        mainHandler.post {
            if (overlayAttached) applyOverlayPresentation()
            mainHandler.postDelayed({ if (overlayAttached) applyOverlayPresentation() }, 250)
        }
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
        if (followsKeyboard() && !isDictationActive() && !imeLayoutKnown() && !imeBoundsSettled()) {
            if (Log.isLoggable(TAG, Log.DEBUG)) Log.d(TAG, "overlay waiting for keyboard bounds ime=${imeBoundsPx()}")
            return
        }
        try {
            val view = VerenuOverlayView(this).apply {
                listener = this@VerenuAccessibilityService
                setDark(resolveDark())
                setCompact(isDocked())
                refreshCover(this, preferPrediction = true)
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
            if (Log.isLoggable(TAG, Log.DEBUG)) Log.d(TAG, "overlay attached cover=$coverSizePx")
            if (Log.isLoggable(TAG, Log.DEBUG)) {
                view.postDelayed({ Log.d(TAG, "overlay ${view.debugDescribe()}") }, 700L)
            }
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
        hideScheduled = false
        overlayMoveAnimator?.cancel()
        dragging = false
        hideSnoozeTarget()
        if (overlayState == VerenuOverlayView.State.CANCELLED) overlayState = VerenuOverlayView.State.IDLE
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
        refreshOverlayVisibility()
        val view = overlay ?: return
        if (next == VerenuOverlayView.State.ERROR && overlayErrorMessage.isNotEmpty()) {
            view.setError(overlayErrorMessage, retry = errorAction != ErrorAction.DISMISS)
        } else {
            view.updateState(next)
        }
    }

    private fun showOverlayError(
        message: String,
        action: ErrorAction = ErrorAction.RETRY_TRANSCRIPTION,
    ) {
        val safe = message.trim().ifEmpty { "Something went wrong" }
        overlayErrorMessage = safe.take(240)
        errorAction = action
        setOverlayState(VerenuOverlayView.State.ERROR)
    }

    override fun onPillTap() {
        when (overlayState) {
            VerenuOverlayView.State.IDLE -> startDictation()
            VerenuOverlayView.State.CANCELLED -> Unit
            VerenuOverlayView.State.RECORDING -> stopDictation()
            VerenuOverlayView.State.ERROR -> retryDictation()
            else -> Unit
        }
    }

    private val holdRelease = VerenuHoldRelease()

    override fun onPillHoldStart() {
        if (holdRelease.holdStart(overlayState == VerenuOverlayView.State.IDLE)) startDictation()
    }

    override fun onPillHoldEnd() {
        if (holdRelease.holdEnd(overlayState == VerenuOverlayView.State.RECORDING)) stopDictation()
    }

    override fun onPillCancel() = requestCancel()

    override fun onPillDragStart() {
        if (overlayAttached.not()) return
        dragging = true
        overlayMoveAnimator?.cancel()
        if (!isDictationActive()) showSnoozeTarget()
    }

    override fun onPillDragMove(rawX: Int, rawY: Int) {
        if (!dragging) return
        val view = overlay ?: return
        val params = overlayParams ?: return
        params.gravity = Gravity.TOP or Gravity.LEFT
        params.x = rawX - view.width / 2
        params.y = rawY - view.height / 2
        applyLayout(view, params)
        snoozeTarget?.let {
            val hot = isOverSnoozeTarget(rawX, rawY)
            it.scaleX = if (hot) 1.08f else 1f
            it.scaleY = if (hot) 1.08f else 1f
            it.alpha = if (hot) 1f else 0.82f
        }
    }

    override fun onPillDragEnd(rawX: Int, rawY: Int) {
        if (!dragging) return
        val inside = isOverSnoozeTarget(rawX, rawY)
        hideSnoozeTarget()
        if (inside) {
            dragging = false
            snoozeFor(SNOOZE_MS)
            return
        }
        val snap = VerenuPillPlacement.snapFor(rawX, rawY, dropContext())
        val unchanged = snap == null || snap.position == placementFor(snap.target)
        if (snap != null && !unchanged) adoptSnap(snap)
        glideFromDrag()
        dragging = false
    }

    private fun placementFor(target: String) =
        if (target == VerenuPillPlacement.TARGET_DOCK) pillDockPosition else pillPosition

    private fun dropContext(): VerenuPillPlacement.DropContext {
        val ime = if (keyboardVisible) imeBoundsPx() else null
        return VerenuPillPlacement.DropContext(
            screenWidth = realScreenWidthPx(),
            screenHeight = realScreenHeightPx(),
            docked = isDocked(),
            position = pillPosition,
            coveringKeyboardMic = coverBounds != null,
            keyboardLeft = ime?.left,
            keyboardTop = ime?.top,
            keyboardRight = ime?.right,
            keyboardReach = (96 * resources.displayMetrics.density).toInt(),
        )
    }

    private fun adoptSnap(snap: VerenuPillPlacement.Snap) {
        val previous = placementFor(snap.target)
        setPlacement(snap.target, snap.position)
        Toast.makeText(this, "Pill moved: ${VerenuPillPlacement.label(snap.position)}", Toast.LENGTH_SHORT).show()
        val handler = requester ?: return
        pendingPlacementSaves.incrementAndGet()
        handler.post {
            var saved = false
            try {
                saved = bridge.savePillPosition(snap.target, snap.position)
            } catch (e: Exception) {
                Log.w(TAG, "pill position save failed", e)
            } finally {
                pendingPlacementSaves.decrementAndGet()
            }
            if (!saved) {
                mainHandler.post {
                    if (placementFor(snap.target) == snap.position) setPlacement(snap.target, previous)
                    Toast.makeText(this, "Couldn't save the pill position", Toast.LENGTH_SHORT).show()
                    if (overlayAttached) repositionOverlay()
                }
            }
        }
    }

    private fun setPlacement(target: String, position: String) {
        if (target == VerenuPillPlacement.TARGET_DOCK) pillDockPosition = position else pillPosition = position
        savePlacementPrefs()
    }

    private fun glideFromDrag() {
        val view = overlay ?: return
        val params = overlayParams ?: return
        val screenWidth = realScreenWidthPx()
        val screenHeight = realScreenHeightPx()
        val width = view.width
        val height = view.height
        val fromX = absoluteLeft(params, width, screenWidth)
        val fromY = absoluteTop(params, height, screenHeight)
        val target = WindowManager.LayoutParams()
        target.copyFrom(params)
        dragging = false
        placeOverlay(target)
        val toX = absoluteLeft(target, width, screenWidth)
        val toY = absoluteTop(target, height, screenHeight)
        overlayMoveAnimator?.cancel()
        gliding = true
        overlayMoveAnimator = ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 160
            addUpdateListener {
                val f = it.animatedValue as Float
                params.gravity = Gravity.TOP or Gravity.LEFT
                params.x = (fromX + (toX - fromX) * f).toInt()
                params.y = (fromY + (toY - fromY) * f).toInt()
                applyLayout(view, params)
            }
            addListener(object : android.animation.AnimatorListenerAdapter() {
                override fun onAnimationEnd(animation: android.animation.Animator) {
                    gliding = false
                    if (!dragging && overlay === view) {
                        placeOverlay(params)
                        applyLayout(view, params)
                    }
                }
            })
            start()
        }
    }

    private fun absoluteLeft(params: WindowManager.LayoutParams, width: Int, screenWidth: Int): Int =
        when (params.gravity and Gravity.RELATIVE_HORIZONTAL_GRAVITY_MASK) {
            Gravity.START, Gravity.LEFT -> params.x
            Gravity.END, Gravity.RIGHT -> screenWidth - width - params.x
            else -> (screenWidth - width) / 2 + params.x
        }

    private fun absoluteTop(params: WindowManager.LayoutParams, height: Int, screenHeight: Int): Int =
        when (params.gravity and Gravity.VERTICAL_GRAVITY_MASK) {
            Gravity.TOP -> params.y
            Gravity.BOTTOM -> screenHeight - height - params.y
            else -> (screenHeight - height) / 2 + params.y
        }

    private fun snoozeFor(durationMs: Long) {
        snoozedUntilMs = SystemClock.elapsedRealtime() + durationMs
        Toast.makeText(this, "Verenu hidden for ${durationMs / 60_000L} minutes", Toast.LENGTH_SHORT).show()
        mainHandler.removeCallbacks(hideOverlayRunnable)
        hideOverlay()
        mainHandler.postDelayed({ refreshOverlayVisibility() }, durationMs + 500L)
    }

    private fun isOverSnoozeTarget(x: Int, y: Int): Boolean {
        val slop = (24 * resources.displayMetrics.density).toInt()
        return snoozeTargetRect.width() > 0 &&
            x in (snoozeTargetRect.left - slop)..(snoozeTargetRect.right + slop) &&
            y in (snoozeTargetRect.top - slop)..(snoozeTargetRect.bottom + slop)
    }

    private fun showSnoozeTarget() {
        if (snoozeTarget != null) return
        val density = resources.displayMetrics.density
        val label = android.widget.TextView(this).apply {
            text = "Drop here to hide for 15 min"
            setTextColor(0xFFFFFFFF.toInt())
            textSize = 14f
            gravity = Gravity.CENTER
            setPadding((22 * density).toInt(), (14 * density).toInt(), (22 * density).toInt(), (14 * density).toInt())
            background = android.graphics.drawable.GradientDrawable().apply {
                cornerRadius = 40 * density
                setColor(0xE6141312.toInt())
                setStroke((1 * density).toInt(), 0x33FFFFFF)
            }
            alpha = 0.82f
        }
        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
                WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or
                WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.CENTER_HORIZONTAL
            y = statusBarHeightPx() + (24 * density).toInt()
        }
        try {
            windowManager.addView(label, params)
            snoozeTarget = label
            label.post {
                val loc = IntArray(2)
                label.getLocationOnScreen(loc)
                snoozeTargetRect.set(loc[0], loc[1], loc[0] + label.width, loc[1] + label.height)
            }
        } catch (e: Exception) {
            Log.w(TAG, "cannot show snooze target", e)
        }
    }

    private fun hideSnoozeTarget() {
        val target = snoozeTarget ?: return
        snoozeTarget = null
        snoozeTargetRect.setEmpty()
        try {
            windowManager.removeView(target)
        } catch (e: Exception) {
            Log.w(TAG, "cannot remove snooze target", e)
        }
    }

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

    override fun onPillResized() {
        if (overlayAttached) repositionOverlay()
    }

    override fun onPillRestart() {
        if (overlayState != VerenuOverlayView.State.CANCELLED) return
        setOverlayState(VerenuOverlayView.State.IDLE)
        startDictation()
    }

    override fun onPillDismiss() {
        if (!isDictationActive() &&
            errorAction != ErrorAction.RETRY_STOP &&
            errorAction != ErrorAction.RETRY_CANCEL
        ) {
            overlayErrorMessage = ""
            setOverlayState(VerenuOverlayView.State.IDLE)
            if (!shouldShowOverlay()) hideOverlay()
        }
    }

    private var lastBrowserSite: Triple<String, String, Long>? = null
    private var lastBrowserScanMs = 0L

    private fun readBrowserDomain(pkg: String): String {
        if (!BROWSER_URL_BAR_IDS.containsKey(pkg)) return ""
        val live = scanAddressBar(pkg)
        if (live.isNotEmpty()) {
            lastBrowserSite = Triple(pkg, live, SystemClock.elapsedRealtime())
            return live
        }
        val seen = lastBrowserSite
        return if (seen != null && seen.first == pkg &&
            SystemClock.elapsedRealtime() - seen.third < BROWSER_SITE_TTL_MS
        ) seen.second else ""
    }

    private fun rememberBrowserDomain(pkg: String) {
        val now = SystemClock.elapsedRealtime()
        if (now - lastBrowserScanMs < BROWSER_SCAN_INTERVAL_MS) return
        lastBrowserScanMs = now
        val host = scanAddressBar(pkg)
        if (host.isNotEmpty()) lastBrowserSite = Triple(pkg, host, now)
    }

    private fun scanAddressBar(pkg: String): String {
        val ids = BROWSER_URL_BAR_IDS[pkg] ?: return ""
        try {
            val root = browserApplicationRoot(pkg) ?: return ""
            try {
                for (id in ids) {
                    val nodes = root.findAccessibilityNodeInfosByViewId("$pkg:id/$id") ?: continue
                    var found = ""
                    for (node in nodes) {
                        try {
                            if (found.isEmpty()) {
                                val raw = node.text?.toString()?.trim().orEmpty()
                                found = hostFromAddressText(raw)
                            }
                        } finally {
                            node.recycle()
                        }
                    }
                    if (found.isNotEmpty()) return found
                }
            } finally {
                root.recycle()
            }
        } catch (e: Exception) {
            Log.w(TAG, "address bar unreadable", e)
        }
        return ""
    }

    private fun browserApplicationRoot(pkg: String): AccessibilityNodeInfo? {
        val active = rootInActiveWindow
        if (active?.packageName?.toString() == pkg) return active
        active?.recycle()

        val interactiveWindows = windows
        try {
            for (window in interactiveWindows) {
                if (window.type != AccessibilityWindowInfo.TYPE_APPLICATION) continue
                val root = window.root ?: continue
                if (root.packageName?.toString() == pkg) return root
                root.recycle()
            }
        } finally {
            interactiveWindows.forEach { it.recycle() }
        }
        return null
    }

    private fun startDictation() {
        if (!holdRelease.beginStart()) return
        val pkg = foregroundPackage
        val editable = hasEditableFocus
        val setText = supportsSetText
        val handler = requester
        if (handler == null) {
            holdRelease.startFinished(recording = false)
            showOverlayError("Could not start recording", ErrorAction.RETRY_START)
            return
        }
        val domain = readBrowserDomain(pkg)
        if (Log.isLoggable(TAG, Log.DEBUG)) Log.d(TAG, "dictation target browser=${BROWSER_URL_BAR_IDS.containsKey(pkg)} siteFound=${domain.isNotEmpty()}")
        handler.post {
            val backendUp = ensureBackendRunning(waitMs = BACKEND_START_WAIT_MS)
            if (!backendUp) {
                mainHandler.post {
                    holdRelease.startFinished(recording = false)
                    showOverlayError("Verenu is still starting — try again", ErrorAction.RETRY_START)
                }
                return@post
            }
            if (!startDictationService()) {
                mainHandler.post {
                    holdRelease.startFinished(recording = false)
                    showOverlayError("Could not start the microphone", ErrorAction.RETRY_START)
                }
                return@post
            }
            val resp = try {
                bridge.startRecording(pkg, editable, setText, domain)
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
                    if (holdRelease.startFinished(recording = true)) stopDictation()
                } else {
                    holdRelease.startFinished(recording = false)
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
                    stopDictationService()
                    setOverlayState(VerenuOverlayView.State.TRANSCRIBING)
                } else {
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

    private fun focusedEditable(
        budget: VerenuInsertionBudget = VerenuInsertionBudget(SystemClock::elapsedRealtime, 350L),
    ): AccessibilityNodeInfo? {
        return try {
            val focus = findInputFocus(budget)
            if (focus != null && focus.isEditable) focus else {
                focus?.recycle()
                null
            }
        } catch (e: Exception) {
            Log.w(TAG, "focus read failed", e)
            null
        }
    }

    private fun performInsertion(
        node: AccessibilityNodeInfo,
        budget: VerenuInsertionBudget,
        seq: Long,
        dictated: String,
    ): Pair<String, Boolean> {
        if (node.isPassword) return Pair("blocked_password", false)
        if (budget.expired) return Pair("clipboard_fallback", false)
        val actions = node.actionList.map { it.id }
        val showingHint = Build.VERSION.SDK_INT >= 26 && node.isShowingHintText
        val readable = node.text != null || showingHint
        val before = if (showingHint) "" else node.text?.toString() ?: ""
        val selKnown = showingHint || node.textSelectionStart >= 0
        val selStart = if (showingHint) 0 else node.textSelectionStart.takeIf { it >= 0 } ?: before.length
        val selEnd = if (showingHint) 0 else node.textSelectionEnd.takeIf { it >= 0 } ?: selStart
        val from = minOf(selStart, selEnd).coerceIn(0, before.length)
        val to = maxOf(selStart, selEnd).coerceIn(0, before.length)

        val reliable = readable && selKnown
        val text = bridge.formatInsertion(
            seq,
            before.substring(0, from).takeLast(CONTEXT_CHARS),
            before.substring(to).take(CONTEXT_CHARS),
            reliable,
            reliable,
            timeoutMs = budget.remainingMs.coerceAtMost(500L).toInt(),
        ) ?: dictated

        if (budget.expired) return Pair("clipboard_fallback", false)
        val edit = VerenuInsertionEdit(before, from, to, text)
        // SET_TEXT and PASTE can both report success while Android silently
        // truncates the edit (Samsung launcher search has a 100-unit limit).
        // Preserve the field and copy the full dictation instead of leaving
        // a partial insertion that the user might then paste again.
        if (!edit.fits(node.maxTextLength, node.viewIdResourceName)) return Pair("blocked_length", false)
        val merged = edit.text
        if (!actions.contains(AccessibilityNodeInfo.ACTION_SET_TEXT)) {
            return Pair("clipboard_fallback", pasteAtCaret(node, budget, text, before, merged))
        }
        val set = node.performAction(
            AccessibilityNodeInfo.ACTION_SET_TEXT,
            Bundle().apply {
                putCharSequence(
                    AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, merged,
                )
            },
        )
        val confirmation = if (set) confirmInsertion(node, budget, before, merged)
            else if (budget.expired) VerenuInsertionBudget.Confirmation.UNAVAILABLE
            else VerenuInsertionBudget.Confirmation.UNCHANGED
        if (confirmation == VerenuInsertionBudget.Confirmation.LANDED) {
            val cursor = edit.cursor
            if (!budget.expired) node.performAction(
                AccessibilityNodeInfo.ACTION_SET_SELECTION,
                Bundle().apply {
                    putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, cursor)
                    putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, cursor)
                },
            )
            return Pair("direct_accessibility", true)
        }
        if (confirmation == VerenuInsertionBudget.Confirmation.UNAVAILABLE) {
            return Pair("clipboard_fallback", false)
        }
        return Pair("clipboard_fallback", pasteAtCaret(node, budget, text, before, merged))
    }

    private fun confirmInsertion(
        node: AccessibilityNodeInfo,
        budget: VerenuInsertionBudget,
        before: String,
        expected: String,
    ): VerenuInsertionBudget.Confirmation = budget.confirm(
        before,
        expected,
        refreshText = {
            if (node.refresh()) {
                if (Build.VERSION.SDK_INT >= 26 && node.isShowingHintText) ""
                else node.text?.toString()
            } else null
        },
        pause = { Thread.sleep(50L) },
    )

    private fun pasteAtCaret(
        node: AccessibilityNodeInfo,
        budget: VerenuInsertionBudget,
        text: String,
        before: String,
        expected: String,
    ): Boolean {
        return try {
            if (budget.expired) return false
            val clipboard = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("Verenu dictation", text))
            val pasted = node.performAction(AccessibilityNodeInfo.ACTION_PASTE)
            val landed = pasted && confirmInsertion(node, budget, before, expected) ==
                VerenuInsertionBudget.Confirmation.LANDED
            if (landed) clipboard.clearPrimaryClip()
            landed
        } catch (e: Exception) {
            Log.w(TAG, "paste at caret failed", e)
            false
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

    private fun schedulePoll(delayMs: Long) {
        poller?.postDelayed({ poll() }, delayMs)
    }

    private fun poll() {
        syncDeviceLockState()
        var syncMutingEnabled = false
        try {
            val snapshot = bridge.getState()
            syncMutingEnabled = snapshot?.syncMutingEnabled == true
            audioMuteActive = snapshot?.audioMuteRequested ?: false
            val up = snapshot != null
            if (up && !backendWasUp) {
                hydrateCredentials()
            }
            backendWasUp = up
            if (snapshot != null) {
                onBridgeState(snapshot)
            } else if (!keyboardVisible) {
                ensureBackendRunning(waitMs = 0L)
            }
        } catch (e: Exception) {
            audioMuteActive = false
            Log.w(TAG, "bridge poll failed", e)
        }
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
            nextPollDelayMs(
                recording = overlayState == VerenuOverlayView.State.RECORDING,
                audioMuteActive = audioMuteActive,
                overlayAttached = overlayAttached,
                syncMutingEnabled = syncMutingEnabled,
            ),
        )
    }

    private fun onBridgeState(snapshot: BridgeStateSnapshot) {
        VerenuAnalytics.setEnabled(snapshot.analyticsEnabled)
        applyPlacementSettings(snapshot)
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

        val bridgeError = snapshot.lastError
        if (bridgeError != null && isDictationActive() && bridgeError.ageMs < 120_000L) {
            VerenuAnalytics.pipelineFailed(currentRunId, bridgeError.message, snapshot.pillStage)
            stopDictationService()
            mainHandler.post {
                showOverlayError(bridgeError.message)
            }
            return
        }

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
                setOverlayState(VerenuOverlayView.State.TRANSCRIBING)
            }

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

        val current = snapshot.pendingInsertion
        if (current == null) {
            lastInsertedSeq = -1L
            lastInsertedPackage = ""
            return
        }
        if (current.seq != lastSeenPendingSeq) {
            lastSeenPendingSeq = current.seq
            lastInsertAttemptMs = 0L
        }
        if (current.seq == lastInsertedSeq) {
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
        val lookupStartedAt = SystemClock.elapsedRealtime()
        val canAttempt = now - lastInsertAttemptMs > INSERT_ATTEMPT_MS
        if (!canAttempt) return

        val budget = VerenuInsertionBudget(SystemClock::elapsedRealtime)
        val node = focusedEditable(budget)
        if (node != null && insertionTargetAvailable(node)) {
            lastInsertAttemptMs = now
            val insertedPackage = node.packageName?.toString().orEmpty()
            VerenuAnalytics.insertionAttempted(currentRunId)
            val (strategy, ok) = try {
                performInsertion(node, budget, current.seq, current.text)
            } catch (e: Exception) {
                Log.w(TAG, "insertion target unavailable", e)
                Pair("clipboard_fallback", false)
            } finally {
                node.recycle()
            }
            val blockedPassword = strategy == "blocked_password"
            val lengthLimited = strategy == "blocked_length"
            val manualCopy = !ok && !blockedPassword &&
                copyToClipboard(current.text, tryPaste = false)
            val ack = bridge.ackInsertion(
                current.seq,
                ok,
                strategy,
                if (blockedPassword) "password_field" else if (lengthLimited) "field_limit" else null,
                insertedPackage,
                discard = blockedPassword || manualCopy,
            )
            if (ok || manualCopy) {
                VerenuAnalytics.dictationInserted(
                    currentRunId,
                    if (ok) strategy else "clipboard_fallback",
                    current.text.trim().split(Regex("\\s+")).count { it.isNotEmpty() },
                )
                if (!ok && manualCopy) VerenuAnalytics.fallbackUsed(currentRunId, "manual_copy")
                lastInsertedSeq = current.seq
                lastInsertedStrategy = if (ok) strategy else "clipboard_fallback"
                lastInsertedPackage = insertedPackage
                lastInsertionAckAttemptMs = System.currentTimeMillis()
                stopDictationService()
                if (ok) {
                    setOverlayState(VerenuOverlayView.State.IDLE)
                } else {
                    mainHandler.post {
                        showOverlayError(
                            if (lengthLimited) "Too long for this field — copied instead"
                            else "Couldn't insert — copied instead",
                            ErrorAction.DISMISS,
                        )
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
        node?.recycle()

        val waitedMs = current.ageMs + (SystemClock.elapsedRealtime() - lookupStartedAt)
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

    private fun insertionTargetAvailable(node: AccessibilityNodeInfo): Boolean {
        val owner = node.packageName?.toString().orEmpty()
        return VerenuFocusPackages.isInsertionField(owner, defaultImePackage())
    }
}
