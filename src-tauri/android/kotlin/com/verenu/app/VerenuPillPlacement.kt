package com.verenu.app

/**
 * Where the pill can rest, and which of those places a drag lands on. Pure
 * logic with no Android types so it runs in plain unit tests.
 *
 * The names mirror `ANDROID_PILL_POSITIONS` / `ANDROID_PILL_SCREEN_POSITIONS`
 * in `src-tauri/src/android/mod.rs`; Rust rejects anything outside them.
 */
internal object VerenuPillPlacement {
    const val DEFAULT_POSITION = "keyboard-center"
    /** Where the pill rests when there is no keyboard to sit on. */
    const val DEFAULT_DOCK_POSITION = "screen-bottom"

    const val TARGET_POSITION = "position"
    const val TARGET_DOCK = "dock"

    private val SCREEN_GRID = arrayOf(
        arrayOf("screen-top-left", "screen-top", "screen-top-right"),
        arrayOf("screen-left", "screen-middle", "screen-right"),
        arrayOf("screen-bottom-left", "screen-bottom", "screen-bottom-right"),
    )
    private val KEYBOARD_ROW = arrayOf("keyboard-left", "keyboard-center", "keyboard-right")

    fun isScreenPosition(position: String) =
        position == "punch-hole" || SCREEN_GRID.any { row -> position in row }

    fun isKnownPosition(position: String) =
        position in KEYBOARD_ROW || isScreenPosition(position)

    fun followsKeyboard(position: String) = position in KEYBOARD_ROW

    /** A stored value this build does not know falls back rather than misplacing the pill. */
    fun sanitizePosition(position: String?): String =
        position?.takeIf { isKnownPosition(it) } ?: DEFAULT_POSITION

    fun sanitizeDockPosition(position: String?): String =
        position?.takeIf { isScreenPosition(it) } ?: DEFAULT_DOCK_POSITION

    fun label(position: String): String = when (position) {
        "keyboard-left" -> "above the keyboard, left"
        "keyboard-center" -> "above the keyboard, center"
        "keyboard-right" -> "above the keyboard, right"
        "screen-top-left" -> "top left"
        "screen-top" -> "top center"
        "screen-top-right" -> "top right"
        "screen-left" -> "middle left"
        "screen-middle" -> "middle of the screen"
        "screen-right" -> "middle right"
        "screen-bottom-left" -> "bottom left"
        "screen-bottom" -> "bottom center"
        "screen-bottom-right" -> "bottom right"
        "punch-hole" -> "under the camera"
        else -> position
    }

    /** The screen position whose third of the display contains the point. */
    fun screenCell(x: Int, y: Int, screenWidth: Int, screenHeight: Int): String {
        val col = third(x, screenWidth)
        val row = third(y, screenHeight)
        return SCREEN_GRID[row][col]
    }

    /** Left, center or right of the keyboard, by which third of its width the point is in. */
    fun keyboardCell(x: Int, keyboardLeft: Int, keyboardRight: Int): String =
        KEYBOARD_ROW[third(x - keyboardLeft, keyboardRight - keyboardLeft)]

    private fun third(value: Int, extent: Int): Int {
        if (extent <= 0) return 1
        return (value * 3 / extent).coerceIn(0, 2)
    }

    /** The setting a drop updates, and the position it snaps to. */
    data class Snap(val target: String, val position: String)

    /** What the pill is doing at the moment it is dropped. */
    data class DropContext(
        val screenWidth: Int,
        val screenHeight: Int,
        /** No keyboard to sit on: a dictation outlived it. */
        val docked: Boolean,
        /** `android_pill_position` while the pill is not docked. */
        val position: String,
        /** Sitting on the keyboard's own mic key, which is deliberately fixed. */
        val coveringKeyboardMic: Boolean,
        val keyboardLeft: Int? = null,
        val keyboardTop: Int? = null,
        val keyboardRight: Int? = null,
        /** How far above the keyboard a drop still counts as "on the keyboard". */
        val keyboardReach: Int = 0,
    )

    /**
     * Where a drop at (x, y) snaps, or null when the pill should just glide
     * back. The setting that currently governs the pill is the one updated:
     * the dock while docked, otherwise the position.
     */
    fun snapFor(x: Int, y: Int, ctx: DropContext): Snap? {
        if (ctx.coveringKeyboardMic && !ctx.docked) return null
        val screen = screenCell(x, y, ctx.screenWidth, ctx.screenHeight)
        if (ctx.docked && followsKeyboard(ctx.position)) return Snap(TARGET_DOCK, screen)
        if (!followsKeyboard(ctx.position)) return Snap(TARGET_POSITION, screen)
        val left = ctx.keyboardLeft
        val top = ctx.keyboardTop
        val right = ctx.keyboardRight
        // "On the keyboard" is both above its top edge by a little and across its
        // width: a floating, split or one-handed keyboard leaves screen to either side.
        if (left != null && top != null && right != null &&
            y >= top - ctx.keyboardReach &&
            x >= left - ctx.keyboardReach && x <= right + ctx.keyboardReach
        ) {
            return Snap(TARGET_POSITION, keyboardCell(x, left, right))
        }
        // Dragged well clear of the keyboard: the user means a place on the screen.
        return Snap(TARGET_POSITION, screen)
    }
}
