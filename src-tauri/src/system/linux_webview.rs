//! Memory policy for the two persistent Linux application webviews.

use webkit2gtk::{SettingsExt, WebContextExt, WebViewExt};

pub(super) fn configure(webview: &webkit2gtk::WebView) {
    // Both windows render a single local Svelte document. Navigation between
    // app pages does not need a browser resource cache or back/forward cache.
    if let Some(context) = webview.context() {
        context.set_cache_model(webkit2gtk::CacheModel::DocumentViewer);
    }
    if let Some(settings) = webview.settings() {
        // wry explicitly enables this during construction, so override it
        // after construction even though DocumentViewer also limits caches.
        settings.set_enable_page_cache(false);
    }
}

pub fn configure_window(window: &tauri::WebviewWindow) {
    if let Err(error) = window.with_webview(|platform| configure(&platform.inner())) {
        log::warn!("could not configure Linux webview memory policy: {error}");
    }
}

/// Restricts which part of `window` receives pointer input, in the window's
/// logical (CSS px) coordinates. `None` makes the whole window click-through.
///
/// Tao's `set_ignore_cursor_events` can only toggle "all or nothing", and
/// refuses (by unwrapping) on a window GTK has not realized yet. The Linux
/// pill is a fixed-size transparent window much larger than its capsule, so it
/// needs a real input region. Returns `false` when the surface is not realized
/// yet; the caller re-applies after the window is shown.
///
/// Must run on the GTK main thread.
pub fn set_input_region(window: &tauri::WebviewWindow, rect: Option<[i32; 4]>) -> bool {
    use gtk::cairo::{RectangleInt, Region};
    use gtk::prelude::WidgetExt;

    let Ok(gtk_window) = window.gtk_window() else {
        return false;
    };
    let Some(surface) = gtk_window.window() else {
        return false;
    };
    // An empty region is read as "no shape" by some GDK backends, so
    // click-through is a single transparent pixel in the corner — the same
    // encoding tao uses for `set_ignore_cursor_events(true)`.
    let [x, y, width, height] = rect
        .filter(|[_, _, w, h]| *w > 0 && *h > 0)
        .unwrap_or([0, 0, 1, 1]);
    surface.input_shape_combine_region(
        &Region::create_rectangle(&RectangleInt::new(x, y, width, height)),
        0,
        0,
    );
    true
}
