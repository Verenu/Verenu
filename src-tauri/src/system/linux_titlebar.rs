//! Extended native chrome for the Linux main window.
//!
//! Tao creates a `GtkApplicationWindow` whose sole content child is a vertical
//! `GtkBox`; wry packs the WebKit view into that box. A normal
//! `GtkWindow::set_titlebar` installs `GtkHeaderBar` outside that content child,
//! so GTK subtracts the header allocation before sizing the WebView. We instead
//! make the existing box the main child of a `GtkOverlay` and place a real,
//! textless `GtkHeaderBar` over its upper-right corner. The WebView therefore
//! owns the full window allocation while GTK continues to provide whichever
//! native controls the active compositor can meaningfully support.

use gtk::glib::Cast;
use gtk::prelude::{
    ContainerExt, CssProviderExt, GtkSettingsExt, GtkWindowExt, HeaderBarExt, ObjectExt,
    OverlayExt, WidgetExt,
};
use serde::Serialize;
use tauri::{Emitter, WebviewWindow};

const HEADER_NAME: &str = "verenu-overlay-titlebar";
const OVERLAY_NAME: &str = "verenu-window-overlay";
const FALLBACK_HEIGHT: i32 = 32;
const FALLBACK_RIGHT_INSET: i32 = 54;

#[derive(Clone, Debug, Serialize)]
pub struct WindowChromeCapabilities {
    pub compositor: String,
    pub minimize_supported: bool,
    pub maximize_supported: bool,
    pub close_supported: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleBarMetrics {
    pub height: i32,
    pub left_inset: i32,
    pub right_inset: i32,
    pub scale_factor: f64,
}

fn fallback(dark: bool) -> (&'static str, &'static str) {
    if dark {
        ("#f5f4f0", "#201f1e")
    } else {
        ("#292522", "#f2f2f1")
    }
}

fn widget_allocation(widget: &impl gtk::glib::IsA<gtk::Widget>) -> String {
    let widget = widget.as_ref();
    let allocation = widget.allocation();
    format!(
        "{}#{} x={} y={} w={} h={}",
        widget.type_().name(),
        widget.widget_name(),
        allocation.x(),
        allocation.y(),
        allocation.width(),
        allocation.height()
    )
}

fn log_allocations(
    phase: &str,
    gtk_window: &gtk::ApplicationWindow,
    content: &gtk::Box,
    header: Option<&gtk::HeaderBar>,
) {
    let titlebar = gtk_window
        .titlebar()
        .map(|widget| widget_allocation(&widget))
        .unwrap_or_else(|| "none".to_owned());
    let content_children = content
        .children()
        .iter()
        .map(widget_allocation)
        .collect::<Vec<_>>()
        .join(", ");
    let parent = content
        .parent()
        .map(|widget| format!("{}#{}", widget.type_().name(), widget.widget_name()))
        .unwrap_or_else(|| "none".to_owned());
    let message = format!(
        "linux chrome {phase}: window=[{}] titlebar=[{}] content-parent={} content=[{}] children=[{}] header=[{}]",
        widget_allocation(gtk_window),
        titlebar,
        parent,
        widget_allocation(content),
        content_children,
        header
            .map(widget_allocation)
            .unwrap_or_else(|| "none".to_owned())
    );
    log::info!("{message}");
}

fn find_header(overlay: &gtk::Overlay) -> Option<gtk::HeaderBar> {
    overlay
        .children()
        .into_iter()
        .find(|child| child.widget_name() == HEADER_NAME)
        .and_then(|child| child.downcast::<gtk::HeaderBar>().ok())
}

fn current_overlay(content: &gtk::Box) -> Option<gtk::Overlay> {
    content
        .parent()
        .and_then(|parent| parent.downcast::<gtk::Overlay>().ok())
        .filter(|overlay| overlay.widget_name() == OVERLAY_NAME)
}

pub fn capabilities() -> WindowChromeCapabilities {
    if crate::core::hyprland::session_available() {
        return WindowChromeCapabilities {
            compositor: "Hyprland".to_owned(),
            minimize_supported: false,
            maximize_supported: false,
            close_supported: true,
        };
    }

    let compositor = std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("DESKTOP_SESSION"))
        .unwrap_or_else(|_| "Linux desktop".to_owned());
    WindowChromeCapabilities {
        compositor,
        minimize_supported: true,
        maximize_supported: true,
        close_supported: true,
    }
}

fn decoration_layout(capabilities: &WindowChromeCapabilities) -> String {
    let mut buttons = Vec::new();
    if capabilities.minimize_supported {
        buttons.push("minimize");
    }
    if capabilities.maximize_supported {
        buttons.push("maximize");
    }
    if capabilities.close_supported {
        buttons.push("close");
    }
    format!(":{}", buttons.join(","))
}

fn create_header() -> gtk::HeaderBar {
    let capabilities = capabilities();
    let header = gtk::HeaderBar::new();
    header.set_show_close_button(capabilities.close_supported);
    header.set_has_subtitle(false);
    header.set_title(None::<&str>);
    header.set_decoration_layout(Some(&decoration_layout(&capabilities)));
    header.set_widget_name(HEADER_NAME);
    header.set_halign(gtk::Align::End);
    header.set_valign(gtk::Align::Start);
    header.set_hexpand(false);
    header.set_vexpand(false);
    header
}

/// Reparents the existing Tauri/wry content box into a `GtkOverlay` and adds
/// native GTK window controls as a top-right overlay child.
///
/// This runs on GTK's main thread during Tauri setup, before the frontend asks
/// for theme synchronization. Repeated calls are harmless.
pub fn enable(window: &WebviewWindow) -> Result<(), String> {
    let gtk_window = window.gtk_window().map_err(|error| error.to_string())?;
    let content = window.default_vbox().map_err(|error| error.to_string())?;

    if let Some(overlay) = current_overlay(&content) {
        if let Some(header) = find_header(&overlay) {
            emit_metrics(window, &header);
            return Ok(());
        }
    }

    log_allocations("before", &gtk_window, &content, None);

    gtk_window.set_decorated(false);
    gtk_window.set_titlebar(None::<&gtk::Widget>);

    gtk_window.remove(&content);
    let overlay = gtk::Overlay::new();
    overlay.set_widget_name(OVERLAY_NAME);
    overlay.add(&content);

    let header = create_header();
    overlay.add_overlay(&header);
    overlay.set_overlay_pass_through(&header, false);
    gtk_window.add(&overlay);
    overlay.show_all();

    emit_metrics(window, &header);
    let window_for_log = gtk_window.clone();
    let content_for_log = content.clone();
    let header_for_log = header.clone();
    gtk::glib::idle_add_local_once(move || {
        log_allocations(
            "after-layout",
            &window_for_log,
            &content_for_log,
            Some(&header_for_log),
        );
    });

    Ok(())
}

fn metrics_for(header: &gtk::HeaderBar) -> TitleBarMetrics {
    let (_, natural_width) = header.preferred_width();
    let (_, natural_height) = header.preferred_height();
    TitleBarMetrics {
        height: header
            .allocated_height()
            .max(natural_height)
            .max(FALLBACK_HEIGHT),
        left_inset: 0,
        right_inset: header
            .allocated_width()
            .max(natural_width)
            .max(FALLBACK_RIGHT_INSET),
        scale_factor: f64::from(header.scale_factor()),
    }
}

fn emit_metrics(window: &WebviewWindow, header: &gtk::HeaderBar) {
    let _ = window.emit("verenu:native-titlebar-metrics", metrics_for(header));
}

pub fn metrics(window: &WebviewWindow) -> Result<TitleBarMetrics, String> {
    enable(window)?;
    let content = window.default_vbox().map_err(|error| error.to_string())?;
    let overlay =
        current_overlay(&content).ok_or_else(|| "Linux chrome overlay missing".to_owned())?;
    let header = find_header(&overlay).ok_or_else(|| "Linux header controls missing".to_owned())?;
    Ok(metrics_for(&header))
}

/// Applies Verenu's resolved foreground and hover colors to the native control
/// cluster. The header itself stays transparent because the WebView now paints
/// the sidebar and paper surfaces beneath it.
pub fn apply_theme(
    window: &WebviewWindow,
    dark: bool,
    _surface: Option<String>,
    text: Option<String>,
    _border: Option<String>,
    hover: Option<String>,
    _sidebar_surface: Option<String>,
    _sidebar_width: Option<String>,
) {
    if let Err(error) = enable(window) {
        log::warn!("linux chrome: failed to enable overlay: {error}");
        return;
    }

    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    let Some(settings) = gtk_window.settings() else {
        return;
    };
    settings.set_gtk_application_prefer_dark_theme(dark);

    let (fallback_text, fallback_hover) = fallback(dark);
    let text = text
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback_text.to_owned());
    let hover = hover
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback_hover.to_owned());
    let css = format!(
        "#{HEADER_NAME} {{ background-color: transparent; background-image: none; color: {text}; \
           border: none; border-image: none; border-radius: 0; box-shadow: none; text-shadow: none; \
           min-height: {FALLBACK_HEIGHT}px; padding: 0 6px; }} \
         #{HEADER_NAME} button {{ background-color: transparent; background-image: none; color: {text}; \
           border: none; border-image: none; box-shadow: none; text-shadow: none; \
           min-height: 24px; min-width: 24px; padding: 4px 8px; border-radius: 6px; }} \
         #{HEADER_NAME} button:hover {{ background-color: {hover}; background-image: none; }} \
         #{HEADER_NAME} button:active {{ background-color: {hover}; background-image: none; opacity: 0.8; }}"
    );
    let provider = gtk::CssProvider::new();
    if let Err(error) = provider.load_from_data(css.as_bytes()) {
        log::warn!("linux chrome: CSS failed to parse: {error}");
        return;
    }
    let Some(screen) = WidgetExt::screen(&gtk_window) else {
        return;
    };
    gtk::StyleContext::add_provider_for_screen(
        &screen,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    if let Ok(metrics) = metrics(window) {
        let _ = window.emit("verenu:native-titlebar-metrics", metrics);
    }
}
