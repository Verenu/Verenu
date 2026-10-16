package com.verenu.app

import android.content.pm.ActivityInfo
import android.os.Bundle
import android.content.res.Configuration
import android.graphics.Color
import android.view.View
import android.webkit.JavascriptInterface
import android.webkit.WebView
import android.net.wifi.WifiManager
import android.util.Log
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private var webView: WebView? = null
  @Volatile private var navOverlapPx = 0
  @Volatile private var imeVisible = false
  private var navBarPx = 0
  private var syncMulticastLock: WifiManager.MulticastLock? = null

  override fun onStart() {
    super.onStart()
    // Android filters multicast by default. Nearby sync discovery needs it
    // only while the app is visible; direct paired connections use unicast.
    try {
      val wifi = applicationContext.getSystemService(WIFI_SERVICE) as? WifiManager
      syncMulticastLock = wifi?.createMulticastLock("verenu-sync-discovery")?.apply {
        setReferenceCounted(false)
        acquire()
      }
    } catch (error: RuntimeException) {
      Log.w("VerenuSync", "Nearby discovery multicast unavailable", error)
    }
  }

  override fun onStop() {
    syncMulticastLock?.let { if (it.isHeld) it.release() }
    syncMulticastLock = null
    super.onStop()
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    webView.addJavascriptInterface(object {
      @JavascriptInterface
      fun bottomInsetCssPx(): Float = navOverlapPx / resources.displayMetrics.density

      @JavascriptInterface
      fun imeVisible(): Boolean = this@MainActivity.imeVisible
    }, "VerenuInsets")
    webView.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> updateNavOverlap() }
  }

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
    if (intent?.getBooleanExtra(EXTRA_BACKGROUND_START, false) == true) {
      moveTaskToBack(true)
    }
  }

  companion object {
    const val EXTRA_BACKGROUND_START = "verenu_background_start"
  }

  private fun installSystemBarInsets() {
    val content = findViewById<View>(android.R.id.content) ?: return
    val night = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
      Configuration.UI_MODE_NIGHT_YES
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
      val nowVisible = insets.isVisible(WindowInsetsCompat.Type.ime())
      if (nowVisible != imeVisible) {
        imeVisible = nowVisible
        webView?.post { webView?.evaluateJavascript("window.dispatchEvent(new Event('verenu-ime'))", null) }
      }
      updateNavOverlap()
      view.setPadding(
        baseLeft + bars.left,
        baseTop + bars.top,
        baseRight + bars.right,
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
