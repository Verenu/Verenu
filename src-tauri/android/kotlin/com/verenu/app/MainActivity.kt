package com.verenu.app

import android.os.Bundle
import android.content.res.Configuration
import android.graphics.Color
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    installSystemBarInsets()
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

    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      view.setPadding(
        baseLeft + bars.left,
        baseTop + bars.top,
        baseRight + bars.right,
        // Samsung's WebView viewport already accounts for the navigation bar.
        // Applying it again lifts Verenu's bottom navigation unnecessarily.
        content.paddingBottom,
      )
      insets
    }
    ViewCompat.requestApplyInsets(content)
  }
}
