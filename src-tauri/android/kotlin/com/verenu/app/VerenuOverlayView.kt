package com.verenu.app

import android.content.Context
import android.content.res.Configuration
import android.animation.LayoutTransition
import android.animation.ValueAnimator
import android.graphics.Canvas
import android.graphics.Paint
import android.text.TextUtils
import android.util.AttributeSet
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.TextView
import kotlin.math.sin

/**
 * The Android dictation pill: the touch equivalent of the desktop pill
 * (`src/PillApp.svelte` + `DictationPill.svelte`).
 *
 * Visual identity mirrors the desktop tokens (near-black/white pill,
 * 11px semibold stage labels, 150–220ms motion feel) in both color schemes.
 * Shape is a Dynamic-Island-style rounded rect (18dp radius, not a full
 * stadium) sized and positioned to hug a centered hole-punch camera when
 * the device has one (see [VerenuAccessibilityService.centeredCutout]).
 *
 * States mirror `src/lib/android/overlay.ts`'s mapping of the desktop
 * `pill-state` events: idle → recording → transcribing → cleaning →
 * inserting, plus error (retry) and cancelled. Rendering only — the
 * [Listener] (implemented by [VerenuAccessibilityService]) owns recording,
 * insertion, and bridge calls.
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

    var listener: Listener? = null

    private val dark: Boolean =
        (context.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
            Configuration.UI_MODE_NIGHT_YES

    // Desktop pill tokens (src/theme.css --pill-*), but inverted against the
    // theme: the pill floats over arbitrary apps right above the keyboard, and
    // a white pill on a light keyboard (or black on dark) disappears.
    private val pillBg = if (dark) 0xFFF2F1EF.toInt() else 0xFF111110.toInt()
    private val pillFg = if (dark) 0xFF111110.toInt() else 0xFFFFFFFF.toInt()
    private val pillMuted = if (dark) 0x73111110 else 0x73FFFFFF
    private val pillLine = if (dark) 0x33111110 else 0x33FFFFFF
    private val errorBg = 0xFF351613.toInt()
    private val errorFg = 0xFFFF8F80.toInt()

    private val row: LinearLayout
    private val wave: WaveView
    private val label: TextView
    private val cancelButton: ImageButton
    private val confirmButton: ImageButton
    private val retryButton: ImageButton
    private val dismissButton: ImageButton

    private var state: State = State.IDLE
        set(value) {
            field = value
            render()
        }

    init {
        val density = context.resources.displayMetrics.density
        // A Dynamic-Island-style rounded rect (not a full 999px stadium) —
        // rounded enough to read as "the pill", squared off enough to hug a
        // hole-punch camera without looking like an oversized capsule.
        val radius = 18f * density
        background = android.graphics.drawable.GradientDrawable().apply {
            shape = android.graphics.drawable.GradientDrawable.RECTANGLE
            cornerRadius = radius
            setColor(pillBg)
            setStroke((1f * density).toInt(), pillLine)
        }
        // Shadow without elevation (overlays + elevation = inconsistent).
        setPadding(
            (12 * density).toInt(), (4 * density).toInt(),
            (12 * density).toInt(), (4 * density).toInt(),
        )
        // Native size-morph animation between states (idle/recording/error
        // widths), mirroring the desktop pill's 150-220ms CSS transitions.
        layoutTransition = LayoutTransition().apply { setDuration(180) }

        row = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            layoutTransition = LayoutTransition().apply { setDuration(180) }
        }
        wave = WaveView(context, pillFg).apply {
            layoutParams = LinearLayout.LayoutParams((80 * density).toInt(), (24 * density).toInt())
        }
        label = TextView(context).apply {
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
            typeface = android.graphics.Typeface.create("sans-serif-medium", android.graphics.Typeface.NORMAL)
            setTextColor(pillFg)
            gravity = Gravity.CENTER
            includeFontPadding = false
            maxLines = 2
            ellipsize = TextUtils.TruncateAt.END
        }
        cancelButton = pillButton(android.R.drawable.ic_menu_close_clear_cancel, "Cancel")
        confirmButton = pillButton(0, "Stop and transcribe").apply { setImageDrawable(stopSquare()) }
        retryButton = pillButton(android.R.drawable.stat_notify_sync, "Retry")
        dismissButton = pillButton(android.R.drawable.ic_menu_close_clear_cancel, "Dismiss")

        cancelButton.setOnClickListener { listener?.onPillCancel() }
        confirmButton.setOnClickListener { listener?.onPillTap() }
        retryButton.setOnClickListener { listener?.onPillRetry() }
        dismissButton.setOnClickListener { listener?.onPillDismiss() }

        // The whole pill remains a comfortable tap target (tap = start/stop).
        isClickable = true
        isFocusable = false
        minimumHeight = (40 * density).toInt()
        setOnClickListener { listener?.onPillTap() }

        addView(row, LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.CENTER))
        render()
    }

    /** Solid rounded square, the universal "stop" glyph, in the pill's ink. */
    private fun stopSquare(): android.graphics.drawable.Drawable {
        val density = context.resources.displayMetrics.density
        return android.graphics.drawable.GradientDrawable().apply {
            shape = android.graphics.drawable.GradientDrawable.RECTANGLE
            cornerRadius = 3f * density
            setColor(pillFg)
            setSize((12 * density).toInt(), (12 * density).toInt())
        }
    }

    private fun pillButton(icon: Int, description: String): ImageButton =
        ImageButton(context).apply {
            if (icon != 0) setImageResource(icon)
            contentDescription = description
            background = null
            // Icon stays proportional to the pill's own height (desktop's
            // hf-btn is 18dp in a 30px pill); the whole pill is still
            // clickable, so this isn't the only hit target.
            val px = (34 * context.resources.displayMetrics.density).toInt()
            layoutParams = LinearLayout.LayoutParams(px, px)
            val inset = (8 * context.resources.displayMetrics.density).toInt()
            setPadding(inset, inset, inset, inset)
            setColorFilter(pillFg)
            isFocusable = false
        }

    fun updateState(next: State) {
        state = next
    }

    /** Live mic level 0..1 from the bridge poll; smoothed locally. */
    fun setAudioLevel(level: Float) {
        wave.setLevel(level.coerceIn(0f, 1f))
    }

    fun setError(message: String) {
        label.text = message.ifEmpty { "Something went wrong" }
        updateState(State.ERROR)
    }

    private var lastBg: Int? = null

    private fun render() {
        row.removeAllViews()
        val targetBg = if (state == State.ERROR) errorBg else pillBg
        animateBackgroundTo(targetBg)
        label.setTextColor(if (state == State.ERROR) errorFg else pillFg)
        when (state) {
            State.IDLE -> {
                label.text = "Tap to dictate"
                row.addView(label)
            }
            State.RECORDING -> {
                row.addView(cancelButton)
                row.addView(wave)
                row.addView(confirmButton)
            }
            State.TRANSCRIBING -> {
                label.text = "Transcribing…"
                row.addView(label)
            }
            State.CLEANING -> {
                label.text = "Cleaning…"
                row.addView(label)
            }
            State.INSERTING -> {
                label.text = "Pasting…"
                row.addView(label)
            }
            State.ERROR -> {
                row.addView(dismissButton)
                // Label text was set via setError(); keep it.
                row.addView(label)
                row.addView(retryButton)
            }
            State.CANCELLED -> {
                label.text = "Cancelled"
                row.addView(dismissButton)
                row.addView(label)
            }
        }
        label.maxWidth = (250 * context.resources.displayMetrics.density).toInt()
        invalidate()
    }

    private fun animateBackgroundTo(target: Int) {
        val from = lastBg ?: target
        lastBg = target
        if (from == target) {
            (background as? android.graphics.drawable.GradientDrawable)?.setColor(target)
            return
        }
        ValueAnimator.ofArgb(from, target).apply {
            duration = 180
            addUpdateListener {
                (background as? android.graphics.drawable.GradientDrawable)?.setColor(it.animatedValue as Int)
            }
        }.start()
    }

    /**
     * Twelve-bar mirrored envelope, echoing the desktop pill visualizer's
     * newest-in-the-middle flow at a coarser fidelity (bridge polls deliver
     * ~4 level updates/sec; local smoothing fills the gaps).
     */
    private class WaveView(context: Context, color: Int) : View(context) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            this.color = color
            strokeCap = Paint.Cap.ROUND
            strokeWidth = 3f * context.resources.displayMetrics.density
        }
        private var level = 0f
        private var phase = 0f

        fun setLevel(next: Float) {
            level = level + (next - level) * 0.35f
            phase += 0.6f
            invalidate()
        }

        override fun onDraw(canvas: Canvas) {
            super.onDraw(canvas)
            val bars = 12
            val gap = width / (bars + 1f)
            val centerY = height / 2f
            val maxH = height * 0.46f
            for (i in 0 until bars) {
                // Distance from the middle pair = age, like the desktop model.
                val dist = kotlin.math.abs(i - (bars - 1) / 2f) / (bars / 2f)
                val motion = 0.5f + 0.5f * sin(phase - dist * 2.2f)
                val h = (3f + (maxH - 3f) * (0.15f + 0.85f * level) * (0.35f + 0.65f * motion))
                val x = gap * (i + 1)
                canvas.drawLine(x, centerY - h, x, centerY + h, paint)
            }
        }
    }
}
