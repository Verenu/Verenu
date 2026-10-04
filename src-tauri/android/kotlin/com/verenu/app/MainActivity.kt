package com.verenu.app

import android.content.pm.ActivityInfo
import android.os.Bundle
import android.content.res.Configuration
import android.graphics.Color
import android.view.View
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private var webView: WebView? = null
  @Volatile private var navOverlapPx = 0
  private var navBarPx = 0

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    // Read by the frontend (App.svelte) to pad its bottom nav. Exposed as a
    // bridge instead of pushed into the page so it survives reloads.
    webView.addJavascriptInterface(object {
      @JavascriptInterface
      fun bottomInsetCssPx(): Float = navOverlapPx / resources.displayMetrics.density
    }, "VerenuInsets")
    webView.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> updateNavOverlap() }
  }

  /**
   * How far the WebView extends under the system navigation/gesture bar. Some
   * WebViews already shrink to exclude it (overlap 0); others draw under it.
   */
  private fun updateNavOverlap() {
    val view = webView ?: return
    val root = view.rootView ?: return
    val location = IntArray(2)
    view.getLocationInWindow(location)
    val bottom = location[1] + view.height
    navOverlapPx = (bottom - (root.height - navBarPx)).coerceAtLeast(0)
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Portrait only: Verenu has no landscape layout. (Android 16 ignores this on
    // large screens such as an unfolded foldable.)
    requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
    installSystemBarInsets()
    // The accessibility service starts this activity only to load the Rust
    // backend (Tauri hosts it in this process). Don't leave the UI over the
    // user's app.
    if (intent?.getBooleanExtra(EXTRA_BACKGROUND_START, false) == true) {
      moveTaskToBack(true)
    }
  }

  companion object {
    const val EXTRA_BACKGROUND_START = "verenu_background_start"
  }

  /**
   * Tauri's Android WebView can report zero CSS safe-area values even when the
   * activity is drawing edge-to-edge. Apply the real system-bar and cutout
   * insets to the content root so every frontend surface, including the
   * absolutely-positioned Settings page, starts below the punch hole.
   */
  private fun installSystemBarInsets() {
    val content = findViewById<View>(android.R.id.content) ?: return
    val night = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
      Configuration.UI_MODE_NIGHT_YES
    // The WebView's app surface starts after the top inset. Paint the native
    // inset area with the same paper color so edge-to-edge never exposes the
    // Material theme's default window color as a contrasting strip.
    content.setBackgroundColor(Color.parseColor(if (night) "#161514" else "#fcfcfa"))
    val baseLeft = content.paddingLeft
    val baseTop = content.paddingTop
    val baseRight = content.paddingRight
    val baseBottom = content.paddingBottom

    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      navBarPx = insets.getInsets(WindowInsetsCompat.Type.navigationBars()).bottom
      updateNavOverlap()
      view.setPadding(
        baseLeft + bars.left,
        baseTop + bars.top,
        baseRight + bars.right,
        // Samsung's WebView viewport already accounts for the navigation bar.
        // Applying it again lifts Verenu's bottom navigation unnecessarily.
        // The keyboard is the exception: edge-to-edge disables the platform's
        // adjustResize, so shrink the page by the keyboard's height ourselves.
        // Sheets, forms and search fields then stay above it instead of under.
        if (insets.isVisible(WindowInsetsCompat.Type.ime())) {
          insets.getInsets(WindowInsetsCompat.Type.ime()).bottom
        } else {
          baseBottom
        },
      )
      insets
    }
    ViewCompat.requestApplyInsets(content)
  }
}
