package com.verenu.app

/** The complete edit, including replacement of the current selection. */
internal class VerenuInsertionEdit(before: String, from: Int, to: Int, inserted: String) {
    val text = before.substring(0, from) + inserted + before.substring(to)
    val cursor = from + inserted.length

    fun fits(maxLength: Int, fieldId: String? = null): Boolean {
        // One UI app search truncates at 100 UTF-16 units while reporting -1.
        // Apply that verified fallback only to its specific search control;
        // an advertised limit always takes precedence.
        val limit = if (maxLength >= 0) maxLength
            else if (fieldId == "com.sec.android.app.launcher:id/search_src_text") 100
            else -1
        return limit < 0 || text.length <= limit
    }
}
