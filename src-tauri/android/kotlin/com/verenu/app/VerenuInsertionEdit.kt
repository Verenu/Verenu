package com.verenu.app

internal class VerenuInsertionEdit(before: String, from: Int, to: Int, inserted: String) {
    val text = before.substring(0, from) + inserted + before.substring(to)
    val cursor = from + inserted.length

    fun fits(maxLength: Int, fieldId: String? = null): Boolean {
        val limit = if (maxLength >= 0) maxLength
            else if (fieldId == "com.sec.android.app.launcher:id/search_src_text") 100
            else -1
        return limit < 0 || text.length <= limit
    }
}
