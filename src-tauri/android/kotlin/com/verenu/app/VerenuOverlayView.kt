package com.verenu.app

import android.animation.Animator
import android.animation.AnimatorListenerAdapter
import android.animation.ValueAnimator
import android.content.Context
import android.content.res.Configuration
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.drawable.GradientDrawable
import android.os.SystemClock
import android.text.TextUtils
import android.util.AttributeSet
import android.util.TypedValue
import android.view.Choreographer
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.animation.DecelerateInterpolator
import android.view.animation.LinearInterpolator
import android.view.animation.PathInterpolator
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.exp
import kotlin.math.log10
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.sin

/**
 * The Android dictation pill: the touch equivalent of the desktop pill
 * (`src/PillApp.svelte` + `DictationPill.svelte`).
 *
 * - A near-black capsule in both color schemes (a pale pill vanishes against a
 *   light keyboard, and the dark scheme gets a hairline border so it still
 *   reads against a dark one). [setDark] follows Verenu's own appearance
 *   setting rather than only the OS night mode.
 * - Idle → recording → transcribing → cleaning → inserting, plus error (retry)
 *   and cancelled, mirroring `src/lib/android/overlay.ts`. Between states the
 *   capsule morphs its width and cross-fades its content.
 * - The recording row is the desktop's envelope visualizer: 12 mirrored bars
 *   where distance from the middle means AGE, driven by the recorder's 10 ms
 *   peak envelope and redrawn every display frame (see [WaveView]).
 * - [setCompact] is the docked form used while a dictation continues without
 *   the keyboard: smaller, just the wave, timer and stop button.
 *
 * Rendering only — the [Listener] (implemented by
 * [VerenuAccessibilityService]) owns recording, insertion, and bridge calls.
 */
class VerenuOverlayView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : FrameLayout(context, attrs) {

    interface Listener {
        fun onPillTap()
        fun onPillCancel()
        fun onPillRetry()
        fun onPillDismiss()
    }

    enum class State {
        IDLE, RECORDING, TRANSCRIBING, CLEANING, INSERTING, ERROR, CANCELLED
    }

    private class Palette(
        val bg: Int,
        val border: Int,
        val fg: Int,
        val muted: Int,
        val errorBg: Int,
        val errorFg: Int,
        val recordDot: Int,
    )

    var listener: Listener? = null

    private val density = context.resources.displayMetrics.density
    private fun dp(value: Float) = value * density
    private fun dpi(value: Float) = (value * density).toInt()

    private var dark: Boolean =
        (context.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
            Configuration.UI_MODE_NIGHT_YES
    private var palette = paletteFor(dark)
    private var compact = false

    private var state: State = State.IDLE
    private var errorMessage = "Something went wrong"

    private val pill = FrameLayout(context)
    private val row = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
    }
    private val background = GradientDrawable().apply { shape = GradientDrawable.RECTANGLE }
    private var wave: WaveView? = null
    private var timerView: TextView? = null
    private var recordingStartedAt = 0L

    private var widthAnimator: ValueAnimator? = null
    private var bgAnimator: ValueAnimator? = null
    private var lastBg: Int? = null

    private val timerTick = object : Runnable {
        override fun run() {
            val view = timerView ?: return
            val seconds = ((SystemClock.elapsedRealtime() - recordingStartedAt) / 1000L).toInt()
            view.text = "%d:%02d".format(seconds / 60, seconds % 60)
            postDelayed(this, 250L)
        }
    }

    init {
        // Window margin so the capsule's border isn't clipped by the window edge.
        setPadding(dpi(4f), dpi(4f), dpi(4f), dpi(4f))
        clipChildren = false
        clipToPadding = false

        pill.background = background
        pill.minimumHeight = dpi(44f)
        pill.isClickable = true
        pill.isFocusable = false
        pill.setOnClickListener { listener?.onPillTap() }
        pill.setOnTouchListener { view, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> view.animate().scaleX(0.96f).scaleY(0.96f).setDuration(80).start()
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL ->
                    view.animate().scaleX(1f).scaleY(1f).setDuration(140).start()
            }
            false
        }
        pill.addView(
            row,
            LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.CENTER),
        )
        addView(pill, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.CENTER))
        applyChrome(animated = false)
        render(animated = false)
    }

    // ------------------------------------------------------------------ API

    fun updateState(next: State) {
        if (state == next) return
        state = next
        applyChrome(animated = true)
        render(animated = true)
    }

    fun setError(message: String) {
        errorMessage = message.ifEmpty { "Something went wrong" }
        if (state == State.ERROR) {
            render(animated = false)
        } else {
            updateState(State.ERROR)
        }
    }

    /** Follow Verenu's appearance setting (the service resolves "system"). */
    fun setDark(value: Boolean) {
        if (dark == value) return
        dark = value
        palette = paletteFor(value)
        applyChrome(animated = false)
        render(animated = false)
    }

    /** Docked form: smaller, wave + timer + stop only. */
    fun setCompact(value: Boolean) {
        if (compact == value) return
        compact = value
        pill.minimumHeight = dpi(if (value) 36f else 44f)
        render(animated = true)
    }

    /** 10 ms peak-envelope samples (linear 0..1) from the recorder. */
    fun setEnvelope(samples: FloatArray) {
        wave?.pushEnvelope(samples)
    }

    /** Fallback for a backend without the envelope: one RMS level per poll. */
    fun setAudioLevel(level: Float) {
        wave?.pushLevel(level.coerceIn(0f, 1f))
    }

    fun animateIn() {
        pill.alpha = 0f
        pill.scaleX = 0.88f
        pill.scaleY = 0.88f
        pill.translationY = dp(10f)
        pill.animate()
            .alpha(1f).scaleX(1f).scaleY(1f).translationY(0f)
            .setDuration(190)
            .setInterpolator(PathInterpolator(0.2f, 0.9f, 0.25f, 1f))
            .start()
    }

    fun animateOut(onEnd: () -> Unit) {
        pill.animate()
            .alpha(0f).scaleX(0.92f).scaleY(0.92f).translationY(dp(6f))
            .setDuration(130)
            .setInterpolator(DecelerateInterpolator())
            .setListener(object : AnimatorListenerAdapter() {
                override fun onAnimationEnd(animation: Animator) {
                    pill.animate().setListener(null)
                    onEnd()
                }
            })
            .start()
    }

    override fun onDetachedFromWindow() {
        removeCallbacks(timerTick)
        widthAnimator?.cancel()
        bgAnimator?.cancel()
        wave?.stop()
        super.onDetachedFromWindow()
    }

    // ------------------------------------------------------------- rendering

    private fun paletteFor(dark: Boolean) = Palette(
        bg = if (dark) 0xFF1D1C1B.toInt() else 0xFF141312.toInt(),
        border = if (dark) 0x40FFFFFF else 0x26000000,
        fg = 0xFFF6F5F3.toInt(),
        muted = 0xA6F6F5F3.toInt(),
        errorBg = 0xFF3B1814.toInt(),
        errorFg = 0xFFFFA194.toInt(),
        recordDot = 0xFFFF5A4D.toInt(),
    )

    private fun applyChrome(animated: Boolean) {
        val target = if (state == State.ERROR) palette.errorBg else palette.bg
        background.cornerRadius = dp(40f)
        background.setStroke(dpi(1f), if (state == State.ERROR) 0x33FF8F80 else palette.border)
        val from = lastBg
        lastBg = target
        bgAnimator?.cancel()
        if (!animated || from == null || from == target) {
            background.setColor(target)
            return
        }
        bgAnimator = ValueAnimator.ofArgb(from, target).apply {
            duration = 200
            addUpdateListener { background.setColor(it.animatedValue as Int) }
            start()
        }
    }

    private fun render(animated: Boolean) {
        removeCallbacks(timerTick)
        wave?.stop()
        wave = null
        timerView = null

        val fromWidth = pill.width
        row.removeAllViews()
        val padH = dpi(if (compact) 10f else 14f)
        row.setPadding(padH, 0, padH, 0)

        when (state) {
            State.IDLE -> {
                row.addView(icon(IconView.Kind.MIC, 18f, palette.fg), iconParams(18f))
                row.addView(label("Tap to dictate", palette.fg), gapStart())
            }
            State.RECORDING -> {
                if (!compact) {
                    row.addView(circleButton(IconView.Kind.CLOSE, palette.muted, "Cancel") { listener?.onPillCancel() }, LinearLayout.LayoutParams(dpi(36f), dpi(36f)))
                }
                val dot = View(context).apply {
                    background = GradientDrawable().apply {
                        shape = GradientDrawable.OVAL
                        setColor(palette.recordDot)
                    }
                    val pulse = ValueAnimator.ofFloat(1f, 0.35f).apply {
                        duration = 700
                        repeatMode = ValueAnimator.REVERSE
                        repeatCount = ValueAnimator.INFINITE
                        addUpdateListener { alpha = it.animatedValue as Float }
                    }
                    addOnAttachStateChangeListener(object : OnAttachStateChangeListener {
                        override fun onViewAttachedToWindow(v: View) = pulse.start()
                        override fun onViewDetachedFromWindow(v: View) = pulse.cancel()
                    })
                }
                row.addView(dot, LinearLayout.LayoutParams(dpi(8f), dpi(8f)).apply {
                    marginStart = dpi(if (compact) 0f else 4f)
                })
                val w = WaveView(context, palette.fg, compact).also { wave = it }
                row.addView(w, LinearLayout.LayoutParams(w.preferredWidth(), w.preferredHeight()).apply {
                    marginStart = dpi(10f)
                    marginEnd = dpi(10f)
                })
                val timer = label("0:00", palette.muted).apply {
                    setTextSize(TypedValue.COMPLEX_UNIT_SP, if (compact) 12f else 13f)
                    typeface = android.graphics.Typeface.create("sans-serif-medium", android.graphics.Typeface.NORMAL)
                    minWidth = dpi(30f)
                    gravity = Gravity.CENTER_VERTICAL or Gravity.END
                }
                timerView = timer
                row.addView(timer)
                val stopSize = dpi(if (compact) 30f else 34f)
                row.addView(stopButton(), LinearLayout.LayoutParams(stopSize, stopSize).apply { marginStart = dpi(6f) })
                recordingStartedAt = SystemClock.elapsedRealtime()
                post(timerTick)
                w.start()
            }
            State.TRANSCRIBING -> busyRow("Transcribing…")
            State.CLEANING -> busyRow("Cleaning up…")
            State.INSERTING -> busyRow("Pasting…")
            State.ERROR -> {
                row.addView(circleButton(IconView.Kind.CLOSE, palette.errorFg, "Dismiss") { listener?.onPillDismiss() }, LinearLayout.LayoutParams(dpi(36f), dpi(36f)))
                row.addView(icon(IconView.Kind.ALERT, 18f, palette.errorFg), iconParams(18f, 4f))
                row.addView(
                    label(errorMessage, palette.errorFg).apply {
                        maxLines = 2
                        maxWidth = dpi(220f)
                    },
                    gapStart(8f),
                )
                row.addView(circleButton(IconView.Kind.RETRY, palette.errorFg, "Retry") { listener?.onPillRetry() }, LinearLayout.LayoutParams(dpi(36f), dpi(36f)).apply { marginStart = dpi(4f) })
            }
            State.CANCELLED -> {
                row.addView(icon(IconView.Kind.CLOSE, 16f, palette.muted), iconParams(16f))
                row.addView(label("Cancelled", palette.muted), gapStart(8f))
            }
        }

        morphWidth(fromWidth, animated)
    }

    /** Animate the capsule from its old width to the new content's width. */
    private fun morphWidth(fromWidth: Int, animated: Boolean) {
        widthAnimator?.cancel()
        val params = pill.layoutParams as LayoutParams
        row.measure(
            MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED),
            MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED),
        )
        val target = max(row.measuredWidth, dpi(60f))
        if (!animated || fromWidth <= 0 || fromWidth == target) {
            params.width = LayoutParams.WRAP_CONTENT
            pill.layoutParams = params
            row.alpha = 1f
            return
        }
        row.alpha = 0f
        row.animate().alpha(1f).setDuration(170).setStartDelay(40).start()
        widthAnimator = ValueAnimator.ofInt(fromWidth, target).apply {
            duration = 220
            interpolator = PathInterpolator(0.2f, 0.9f, 0.25f, 1f)
            addUpdateListener {
                params.width = it.animatedValue as Int
                pill.layoutParams = params
            }
            addListener(object : AnimatorListenerAdapter() {
                override fun onAnimationEnd(animation: Animator) {
                    params.width = LayoutParams.WRAP_CONTENT
                    pill.layoutParams = params
                }
            })
            start()
        }
    }

    private fun busyRow(text: String) {
        row.addView(SpinnerView(context, palette.fg), LinearLayout.LayoutParams(dpi(18f), dpi(18f)))
        row.addView(label(text, palette.fg), gapStart(10f))
    }

    private fun label(text: String, color: Int) = TextView(context).apply {
        this.text = text
        setTextSize(TypedValue.COMPLEX_UNIT_SP, if (compact) 12f else 14f)
        typeface = android.graphics.Typeface.create("sans-serif-medium", android.graphics.Typeface.NORMAL)
        setTextColor(color)
        includeFontPadding = false
        maxLines = 1
        ellipsize = TextUtils.TruncateAt.END
    }

    private fun icon(kind: IconView.Kind, @Suppress("UNUSED_PARAMETER") sizeDp: Float, color: Int) =
        IconView(context, kind, color)

    /** Icons have no intrinsic size, so every add must carry explicit params. */
    private fun iconParams(sizeDp: Float, marginStartDp: Float = 0f) =
        LinearLayout.LayoutParams(dpi(sizeDp), dpi(sizeDp)).apply { marginStart = dpi(marginStartDp) }

    private fun gapStart(valueDp: Float = 8f) =
        LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
        ).apply { marginStart = dpi(valueDp) }

    /** A 36dp touch target around a 16dp glyph. */
    private fun circleButton(
        kind: IconView.Kind,
        color: Int,
        description: String,
        onClick: () -> Unit,
    ) = FrameLayout(context).apply {
        contentDescription = description
        isClickable = true
        isFocusable = false
        layoutParams = LinearLayout.LayoutParams(dpi(36f), dpi(36f))
        addView(
            IconView(context, kind, color),
            LayoutParams(dpi(16f), dpi(16f), Gravity.CENTER),
        )
        setOnClickListener { onClick() }
    }

    /** Solid light disc with a dark rounded square: the unmistakable stop. */
    private fun stopButton(): View {
        val size = if (compact) 30f else 34f
        return FrameLayout(context).apply {
            contentDescription = "Stop and transcribe"
            isClickable = true
            isFocusable = false
            layoutParams = LinearLayout.LayoutParams(dpi(size), dpi(size))
            background = GradientDrawable().apply {
                shape = GradientDrawable.OVAL
                setColor(palette.fg)
            }
            addView(
                View(context).apply {
                    background = GradientDrawable().apply {
                        shape = GradientDrawable.RECTANGLE
                        cornerRadius = dp(3f)
                        setColor(palette.bg)
                    }
                },
                LayoutParams(dpi(11f), dpi(11f), Gravity.CENTER),
            )
            setOnClickListener { listener?.onPillTap() }
        }
    }

    // ------------------------------------------------------------------ icons

    private class IconView(context: Context, val kind: Kind, color: Int) : View(context) {
        enum class Kind { MIC, CLOSE, RETRY, ALERT }

        private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            this.color = color
            style = Paint.Style.STROKE
            strokeCap = Paint.Cap.ROUND
            strokeJoin = Paint.Join.ROUND
        }

        override fun onDraw(canvas: Canvas) {
            val s = min(width, height).toFloat()
            paint.strokeWidth = max(1.6f, s * 0.11f)
            val cx = width / 2f
            val cy = height / 2f
            val left = cx - s / 2f
            val top = cy - s / 2f
            when (kind) {
                Kind.MIC -> {
                    canvas.drawRoundRect(
                        RectF(left + s * 0.33f, top + s * 0.06f, left + s * 0.67f, top + s * 0.56f),
                        s * 0.17f, s * 0.17f, paint,
                    )
                    canvas.drawArc(
                        RectF(left + s * 0.2f, top + s * 0.24f, left + s * 0.8f, top + s * 0.8f),
                        0f, 180f, false, paint,
                    )
                    canvas.drawLine(cx, top + s * 0.8f, cx, top + s * 0.94f, paint)
                }
                Kind.CLOSE -> {
                    val inset = s * 0.18f
                    canvas.drawLine(left + inset, top + inset, left + s - inset, top + s - inset, paint)
                    canvas.drawLine(left + s - inset, top + inset, left + inset, top + s - inset, paint)
                }
                Kind.RETRY -> {
                    val oval = RectF(left + s * 0.14f, top + s * 0.14f, left + s * 0.86f, top + s * 0.86f)
                    canvas.drawArc(oval, -40f, 290f, false, paint)
                    // Arrowhead at the arc's start (-40°).
                    val radius = s * 0.36f
                    val angle = Math.toRadians(-40.0)
                    val ax = cx + radius * cos(angle).toFloat()
                    val ay = cy + radius * sin(angle).toFloat()
                    val head = Path().apply {
                        moveTo(ax + s * 0.04f, ay - s * 0.22f)
                        lineTo(ax, ay)
                        lineTo(ax + s * 0.22f, ay + s * 0.02f)
                    }
                    canvas.drawPath(head, paint)
                }
                Kind.ALERT -> {
                    canvas.drawCircle(cx, cy, s * 0.42f, paint)
                    canvas.drawLine(cx, cy - s * 0.2f, cx, cy + s * 0.06f, paint)
                    canvas.drawPoint(cx, cy + s * 0.22f, paint)
                }
            }
        }
    }

    // ---------------------------------------------------------------- spinner

    private class SpinnerView(context: Context, color: Int) : View(context) {
        private val ring = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            this.color = (color and 0x00FFFFFF) or 0x33000000
            style = Paint.Style.STROKE
            strokeCap = Paint.Cap.ROUND
        }
        private val arc = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            this.color = color
            style = Paint.Style.STROKE
            strokeCap = Paint.Cap.ROUND
        }
        private var angle = 0f
        private val animator = ValueAnimator.ofFloat(0f, 360f).apply {
            duration = 850
            interpolator = LinearInterpolator()
            repeatCount = ValueAnimator.INFINITE
            addUpdateListener {
                angle = it.animatedValue as Float
                invalidate()
            }
        }

        override fun onAttachedToWindow() {
            super.onAttachedToWindow()
            animator.start()
        }

        override fun onDetachedFromWindow() {
            animator.cancel()
            super.onDetachedFromWindow()
        }

        override fun onDraw(canvas: Canvas) {
            val stroke = max(2f, width * 0.12f)
            ring.strokeWidth = stroke
            arc.strokeWidth = stroke
            val inset = stroke / 2f + 0.5f
            val rect = RectF(inset, inset, width - inset, height - inset)
            canvas.drawArc(rect, 0f, 360f, false, ring)
            canvas.drawArc(rect, angle, 100f, false, arc)
        }
    }

    // ------------------------------------------------------------------- wave

    /**
     * Port of the desktop envelope visualizer (`src/lib/pillVisualizer.ts`),
     * simplified: 10 ms peak samples → dB → adaptive floor/ceiling → ring
     * buffer. Each bar reads the ring at its own AGE (distance from the middle
     * × 45 ms) behind a playhead that advances with real time, then eases
     * toward that height every display frame — so motion is continuous at the
     * screen's refresh rate even though samples arrive in small batches.
     */
    private class WaveView(context: Context, color: Int, private val compact: Boolean) : View(context) {
        private val density = context.resources.displayMetrics.density
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            this.color = color
            strokeCap = Paint.Cap.ROUND
        }

        private val heights = FloatArray(BARS)
        private val ringValues = FloatArray(RING)
        private val ringTimes = LongArray(RING)
        private var ringCount = 0
        private var ringHead = 0 // next write index
        private var lastAddedNs = 0L

        private var floorDb = -62f
        private var ceilDb = -32f
        private var running = false
        private var lastFrameNs = 0L
        private val frame = object : Choreographer.FrameCallback {
            override fun doFrame(frameTimeNanos: Long) {
                if (!running) return
                step(frameTimeNanos)
                invalidate()
                Choreographer.getInstance().postFrameCallback(this)
            }
        }

        fun preferredWidth() = ((BARS * barWidth() + (BARS - 1) * barGap()).toInt())
        fun preferredHeight() = (maxHeight() + 2 * density).toInt()
        private fun barWidth() = (if (compact) 2.6f else 3f) * density
        private fun barGap() = (if (compact) 2.6f else 3.2f) * density
        private fun minHeight() = 3f * density
        private fun maxHeight() = (if (compact) 17f else 24f) * density

        fun start() {
            if (running) return
            running = true
            lastFrameNs = 0L
            Choreographer.getInstance().postFrameCallback(frame)
        }

        fun stop() {
            running = false
            Choreographer.getInstance().removeFrameCallback(frame)
        }

        fun pushEnvelope(samples: FloatArray) {
            if (samples.isEmpty()) return
            val now = System.nanoTime()
            val n = samples.size
            for (i in 0 until n) {
                val t = now - (n - 1 - i) * SAMPLE_NS
                add(normalize(samples[i]), t)
            }
        }

        fun pushLevel(level: Float) {
            // Coarse fallback: one RMS level per poll, treated as a flat run.
            val now = System.nanoTime()
            for (i in 0 until 6) add(level, now - (5 - i) * SAMPLE_NS)
        }

        private fun add(value: Float, rawTimeNs: Long) {
            // Keep timestamps strictly increasing even when batches overlap.
            val timeNs = max(rawTimeNs, lastAddedNs + 1_000_000L)
            lastAddedNs = timeNs
            ringValues[ringHead] = value
            ringTimes[ringHead] = timeNs
            ringHead = (ringHead + 1) % RING
            if (ringCount < RING) ringCount++
        }

        /** Linear peak → 0..1 against an adaptive noise floor and ceiling. */
        private fun normalize(peak: Float): Float {
            val db = 20f * log10(max(peak, 1e-5f))
            // Floor: falls fast, rises slowly, so it settles on the room.
            val floorRate = if (db < floorDb) 0.033f else 0.0013f
            floorDb += (db - floorDb) * floorRate
            // Ceiling: attacks fast, releases slowly, never closer than 14 dB.
            val ceilRate = if (db > ceilDb) 0.3f else 0.0066f
            ceilDb += (db - ceilDb) * ceilRate
            ceilDb = max(ceilDb, floorDb + 14f)
            val low = floorDb + 3f
            val x = ((db - low) / (ceilDb - low)).coerceIn(0f, 1f)
            return x.pow(0.85f)
        }

        /** Value at [timeNs], interpolated between the surrounding samples. */
        private fun valueAt(timeNs: Long): Float {
            if (ringCount == 0) return 0f
            var newerValue = Float.NaN
            var newerTime = 0L
            // Walk from newest to oldest.
            for (k in 0 until ringCount) {
                val idx = (ringHead - 1 - k + RING * 2) % RING
                val t = ringTimes[idx]
                if (t <= timeNs) {
                    val v = ringValues[idx]
                    if (newerValue.isNaN()) return if (timeNs - t > STALL_NS) 0f else v
                    val span = (newerTime - t).toFloat()
                    val f = if (span <= 0f) 1f else (timeNs - t) / span
                    return v + (newerValue - v) * f
                }
                newerValue = ringValues[idx]
                newerTime = t
            }
            return 0f
        }

        private fun step(frameTimeNanos: Long) {
            val dt = if (lastFrameNs == 0L) 0.016f else ((frameTimeNanos - lastFrameNs) / 1e9f).coerceIn(0.001f, 0.1f)
            lastFrameNs = frameTimeNanos
            val now = System.nanoTime()
            val half = (BARS - 1) / 2f
            val minH = minHeight()
            val maxH = maxHeight()
            for (i in 0 until BARS) {
                val age = abs(i - half) * AGE_STEP_NS
                val v = valueAt(now - LATENCY_NS - age.toLong())
                val target = minH + (maxH - minH) * v
                val rate = if (target > heights[i]) 46f else 17f
                if (heights[i] == 0f) heights[i] = minH
                heights[i] += (target - heights[i]) * (1f - exp(-rate * dt))
            }
        }

        override fun onDraw(canvas: Canvas) {
            val bw = barWidth()
            val gap = barGap()
            paint.strokeWidth = bw
            val cy = height / 2f
            val total = BARS * bw + (BARS - 1) * gap
            var x = (width - total) / 2f + bw / 2f
            for (i in 0 until BARS) {
                val h = max(heights[i], minHeight())
                val half = (h - bw) / 2f
                canvas.drawLine(x, cy - half, x, cy + half, paint)
                x += bw + gap
            }
        }

        companion object {
            const val BARS = 12
            const val RING = 256
            const val SAMPLE_NS = 10_000_000L
            const val AGE_STEP_NS = 45_000_000f
            const val LATENCY_NS = 70_000_000L
            const val STALL_NS = 220_000_000L
        }
    }
}
