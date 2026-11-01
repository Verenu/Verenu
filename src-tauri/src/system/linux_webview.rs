use tauri::Manager;
use webkit2gtk::{SettingsExt, WebContextExt, WebViewExt};

#[derive(Default)]
struct RendererRecovery {
    generation: u64,
    scheduled: Option<u64>,
    destroyed: bool,
    last_reload: Option<std::time::Instant>,
    attempts: u32,
}

impl RendererRecovery {
    fn schedule(&mut self, now: std::time::Instant) -> Option<(u64, std::time::Duration)> {
        if self.destroyed || self.scheduled.is_some() {
            return None;
        }
        if self
            .last_reload
            .is_some_and(|last| now.duration_since(last) >= std::time::Duration::from_secs(60))
        {
            self.attempts = 0;
        }
        self.generation = self.generation.wrapping_add(1);
        self.scheduled = Some(self.generation);
        let delay = if self.attempts == 0 {
            std::time::Duration::from_millis(250)
        } else {
            std::time::Duration::from_secs(1 << self.attempts.min(5))
        };
        Some((self.generation, delay))
    }

    fn cancel(&mut self) {
        self.scheduled = None;
    }

    fn destroy(&mut self) {
        self.destroyed = true;
        self.cancel();
    }

    fn take_scheduled(&mut self, generation: u64, now: std::time::Instant) -> bool {
        if self.destroyed || self.scheduled != Some(generation) {
            return false;
        }
        self.scheduled = None;
        self.last_reload = Some(now);
        self.attempts = self.attempts.saturating_add(1);
        true
    }
}

pub(super) fn configure(webview: &webkit2gtk::WebView) {
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
    let label = window.label().to_owned();
    let app = window.app_handle().clone();
    if let Err(error) = window.with_webview(move |platform| {
        use gtk::prelude::WidgetExt;

        let webview = platform.inner();
        configure(&webview);
        let recovery = std::rc::Rc::new(std::cell::RefCell::new(RendererRecovery::default()));
        let on_load = recovery.clone();
        webview.connect_load_changed(move |_, event| {
            if event == webkit2gtk::LoadEvent::Started {
                on_load.borrow_mut().cancel();
            }
        });
        let on_destroy = recovery.clone();
        webview.connect_destroy(move |_| on_destroy.borrow_mut().destroy());
        webview.connect_web_process_terminated(move |_, reason| {
            log::warn!("Linux webview {label} renderer terminated ({reason:?})");
            let now = std::time::Instant::now();
            let Some((generation, delay)) = recovery.borrow_mut().schedule(now) else {
                return;
            };
            let app = app.clone();
            let label = label.clone();
            let recovery = recovery.clone();
            gtk::glib::timeout_add_local_once(delay, move || {
                if !recovery
                    .borrow_mut()
                    .take_scheduled(generation, std::time::Instant::now())
                {
                    return;
                }
                if let Some(window) = app.get_webview_window(&label) {
                    log::info!("Reloading Linux webview {label} after renderer termination");
                    if let Err(error) = window.reload() {
                        log::error!("could not reload Linux webview {label}: {error}");
                    }
                }
            });
        });
    }) {
        log::warn!("could not configure Linux webview memory policy: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::RendererRecovery;
    use std::time::{Duration, Instant};

    #[test]
    fn recovery_cancels_stale_timers_and_keeps_backoff_per_view() {
        let mut pill = RendererRecovery::default();
        let mut main = RendererRecovery::default();
        let now = Instant::now();
        let (first, delay) = pill.schedule(now).unwrap();
        assert_eq!(delay, Duration::from_millis(250));
        assert!(pill.schedule(now).is_none());
        assert!(pill.take_scheduled(first, now));
        // The load started by automatic recovery must not reset its backoff.
        pill.cancel();
        let (stale, delay) = pill.schedule(now).unwrap();
        assert_eq!(delay, Duration::from_secs(2));
        // A manual navigation invalidates the queued automatic reload.
        pill.cancel();
        let (current, _) = pill.schedule(now).unwrap();
        assert!(!pill.take_scheduled(stale, now));
        assert!(pill.take_scheduled(current, now));

        let (main_timer, delay) = main.schedule(now).unwrap();
        assert_eq!(delay, Duration::from_millis(250));
        main.destroy();
        assert!(!main.take_scheduled(main_timer, now));
        assert!(main.schedule(now).is_none());
        assert_eq!(
            pill.schedule(now + Duration::from_secs(60)).unwrap().1,
            Duration::from_millis(250)
        );
    }
}

/// Restricts which part of `window` receives pointer input, in the window's
/// logical (CSS px) coordinates. `None` makes the whole window click-through.
///
/// Tao's `set_ignore_cursor_events` can only toggle "all or nothing", and
/// refuses (by unwrapping) on a window GTK has not realized yet. The Linux
/// pill needs a real input region that excludes the transparent margin.
/// Returns `false` when the surface is not realized
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
    let region = match rect.filter(|[_, _, w, h]| *w > 0 && *h > 0) {
        Some([x, y, width, height]) => {
            Region::create_rectangle(&RectangleInt::new(x, y, width, height))
        }
        None => Region::create(),
    };
    gtk_window.input_shape_combine_region(Some(&region));
    surface.input_shape_combine_region(&region, 0, 0);
    true
}
