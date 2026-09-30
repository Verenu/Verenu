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
