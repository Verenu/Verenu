//! Opt-in display check. GTK/WebKit must initialize on the process main thread,
//! rather than a Rust test worker, including for clean WebKit shutdown.

#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../src/system/linux_webview.rs"]
mod policy;

fn main() {
    #[cfg(target_os = "linux")]
    {
        use webkit2gtk::{SettingsExt, WebContextExt, WebViewExt};

        gtk::init().expect("GTK display");
        let webview = webkit2gtk::WebView::new();
        let context = webview.context().expect("WebKit context");
        let settings = webview.settings().expect("WebKit settings");
        context.set_cache_model(webkit2gtk::CacheModel::WebBrowser);
        settings.set_enable_page_cache(true);
        policy::configure(&webview);
        assert_eq!(
            context.cache_model(),
            webkit2gtk::CacheModel::DocumentViewer
        );
        assert!(!settings.enables_page_cache());
        println!("Linux WebKit memory policy verified");
    }
}
