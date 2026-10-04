package com.verenu.app

internal object VerenuFocusPackages {
    fun eventTarget(current: String, owner: String, isViewFocus: Boolean, ownPackage: String, imePackage: String?): String =
        if (isViewFocus && isAppWindow(owner, ownPackage, imePackage)) owner else current

    fun isAppWindow(owner: String, ownPackage: String, imePackage: String?): Boolean =
        owner.isNotEmpty() && owner != ownPackage && isInsertionField(owner, imePackage)

    // The focused editable field is authoritative. A notification/privacy
    // window can leave the recording's package hint pointing at System UI.
    fun isInsertionField(owner: String, imePackage: String?): Boolean =
        owner != "com.android.systemui" && owner != "android" && owner != imePackage
}
