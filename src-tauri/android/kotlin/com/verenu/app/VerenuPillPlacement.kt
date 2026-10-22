package com.verenu.app

internal object VerenuPillPlacement {
    const val DEFAULT_POSITION = "keyboard-center"
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

    fun screenCell(x: Int, y: Int, screenWidth: Int, screenHeight: Int): String {
        val col = third(x, screenWidth)
        val row = third(y, screenHeight)
        return SCREEN_GRID[row][col]
    }

    fun keyboardCell(x: Int, keyboardLeft: Int, keyboardRight: Int): String =
        KEYBOARD_ROW[third(x - keyboardLeft, keyboardRight - keyboardLeft)]

    private fun third(value: Int, extent: Int): Int {
        if (extent <= 0) return 1
        return (value * 3 / extent).coerceIn(0, 2)
    }

    data class Snap(val target: String, val position: String)

    data class DropContext(
        val screenWidth: Int,
        val screenHeight: Int,
        val docked: Boolean,
        val position: String,
        val coveringKeyboardMic: Boolean,
        val keyboardLeft: Int? = null,
        val keyboardTop: Int? = null,
        val keyboardRight: Int? = null,
        val keyboardReach: Int = 0,
    )

    fun snapFor(x: Int, y: Int, ctx: DropContext): Snap? {
        if (ctx.coveringKeyboardMic && !ctx.docked) return null
        val screen = screenCell(x, y, ctx.screenWidth, ctx.screenHeight)
        if (ctx.docked && followsKeyboard(ctx.position)) return Snap(TARGET_DOCK, screen)
        if (!followsKeyboard(ctx.position)) return Snap(TARGET_POSITION, screen)
        val left = ctx.keyboardLeft
        val top = ctx.keyboardTop
        val right = ctx.keyboardRight
        if (left != null && top != null && right != null &&
            y >= top - ctx.keyboardReach &&
            x >= left - ctx.keyboardReach && x <= right + ctx.keyboardReach
        ) {
            return Snap(TARGET_POSITION, keyboardCell(x, left, right))
        }
        return Snap(TARGET_POSITION, screen)
    }
}
