pub mod apps;
pub mod connectivity;
pub mod diagnostics;
pub mod icons;
pub mod logger;
#[cfg(target_os = "macos")]
pub mod mac_app;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod linux_proc;
#[cfg(target_os = "linux")]
pub mod linux_titlebar;
pub mod media_control;
pub mod memory;
pub mod notify;
pub mod number_parser;
pub mod platform;
pub mod session;
pub mod text;
pub mod volume;
#[cfg(target_os = "windows")]
pub mod windows_titlebar;
#[cfg(not(target_os = "windows"))]
pub mod windows_titlebar {
    #[cfg(target_os = "linux")]
    #[tauri::command]
    pub fn get_native_titlebar_metrics(
        window: tauri::WebviewWindow,
    ) -> Result<super::linux_titlebar::TitleBarMetrics, String> {
        super::linux_titlebar::metrics(&window)
    }

    #[cfg(not(target_os = "linux"))]
    #[tauri::command]
    pub fn get_native_titlebar_metrics() -> Result<(), String> {
        Err("native title bar metrics are unavailable on this platform".to_owned())
    }

    #[cfg(target_os = "linux")]
    #[tauri::command]
    pub fn set_native_titlebar_theme(
        window: tauri::WebviewWindow,
        dark: bool,
        surface: Option<String>,
        text: Option<String>,
        border: Option<String>,
        hover: Option<String>,
        sidebar_surface: Option<String>,
        sidebar_width: Option<String>,
    ) {
        super::linux_titlebar::apply_theme(
            &window,
            dark,
            surface,
            text,
            border,
            hover,
            sidebar_surface,
            sidebar_width,
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[tauri::command]
    #[allow(clippy::too_many_arguments)]
    pub fn set_native_titlebar_theme(
        _window: tauri::WebviewWindow,
        _dark: bool,
        _surface: Option<String>,
        _text: Option<String>,
        _border: Option<String>,
        _hover: Option<String>,
        _sidebar_surface: Option<String>,
        _sidebar_width: Option<String>,
    ) {
    }
}

/// Stops and unloads both local model engines (LLM + STT) before the process
/// exits. Must run on every path that exits the app while a local model could
/// be loaded — `RunEvent::Exit` and the update-install handoff — because
/// llama-server.exe is a real child process: a plain `std::process::exit` (or
/// a skipped `RunEvent::Exit`) orphans it, leaving it running and holding the
/// loaded model's RAM/VRAM with no owner. The STT engine is in-process, so
/// process teardown would reclaim its memory anyway — it is unloaded
/// explicitly for deterministic ordering and earlier RAM release during a
/// slow teardown.
pub(crate) fn shutdown_local_models(app: &tauri::AppHandle) {
    use std::sync::atomic::Ordering;
    use tauri::Manager;

    let llm = app.state::<crate::local_llm::LocalLlmManager>();
    llm.shutdown_signal.store(true, Ordering::Relaxed);
    llm.unload(app);

    let stt = app.state::<crate::local_stt::LocalTranscriptionManager>();
    stt.shutdown_signal.store(true, Ordering::Relaxed);
    stt.unload(app);
}
