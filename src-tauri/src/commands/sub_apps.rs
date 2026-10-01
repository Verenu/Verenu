//! Sub-app commands and the capture-hotkey snapshot.

use super::*;
use std::sync::Mutex;

pub const SUB_APP_CAPTURED_EVENT: &str = "verenu:sub-app-captured";
pub const SUB_APP_CAPTURE_FAILED_EVENT: &str = "verenu:sub-app-capture-failed";

/// What the capture hotkey saw in the foreground window. The title only
/// travels to the local review sheet; it is not persisted or logged.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SubAppCapture {
    pub executable: String,
    pub app_name: String,
    pub window_title: String,
    pub proposed_pattern: String,
}

static PENDING_CAPTURE: Mutex<Option<SubAppCapture>> = Mutex::new(None);

/// Snapshot of the foreground window, taken before Verenu raises itself.
/// `None` when nothing usable is focused (no title, or Verenu itself).
pub fn capture_foreground() -> Option<SubAppCapture> {
    let target = crate::core::window_geometry::WindowTarget::capture_foreground();
    if target.id == 0 {
        return None;
    }
    let executable = crate::core::window_context::get_process_name_for_hwnd(target.id)?
        .trim()
        .to_lowercase();
    if executable.is_empty() || executable.starts_with("verenu") {
        return None;
    }
    let window_title = crate::core::window_context::get_window_title(target.id)
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())?;
    // The shared cache is empty until its first background scan finishes;
    // this runs off the hotkey loop, so scan directly in that case.
    let (cached, _) = crate::system::apps::list_installed_apps_cached_with_status();
    let scanned;
    let installed: &[crate::system::apps::InstalledApp] = if cached.is_empty() {
        scanned = crate::system::apps::list_installed_apps();
        &scanned
    } else {
        &cached
    };
    let app_name = installed
        .iter()
        .find(|app| app.exe.eq_ignore_ascii_case(&executable))
        .map(|app| app.name.clone())
        .or_else(|| {
            #[cfg(target_os = "linux")]
            return crate::system::icons::linux_app_display_name(&executable);
            #[cfg(not(target_os = "linux"))]
            None
        })
        .unwrap_or_else(|| fallback_app_name(&executable));
    let proposed_pattern = db::propose_title_pattern(&window_title, Some(&app_name));
    Some(SubAppCapture {
        executable,
        app_name,
        window_title,
        proposed_pattern,
    })
}

/// Readable name when no installed app matches: drop `.exe`/`.app` and use
/// the last segment of a reverse-DNS Linux app id (`com.t3tools.t3code`).
fn fallback_app_name(executable: &str) -> String {
    let base = executable.trim_end_matches(".exe").trim_end_matches(".app");
    base.rsplit('.').next().filter(|s| !s.is_empty()).unwrap_or(base).to_string()
}

/// Hotkey entry point: capture, hand the snapshot to the UI, then show it.
pub fn handle_capture_hotkey(app: &AppHandle) {
    match capture_foreground() {
        Some(capture) => {
            if let Ok(mut slot) = PENDING_CAPTURE.lock() {
                *slot = Some(capture.clone());
            }
            log::info!("sub-app: captured foreground window for review");
            let _ = app.emit(SUB_APP_CAPTURED_EVENT, capture);
            crate::app_setup::show_main_window(app);
        }
        None => {
            log::info!("sub-app: capture skipped — no titled foreground window");
            let _ = app.emit(
                SUB_APP_CAPTURE_FAILED_EVENT,
                "Focus the window you want to capture, then press the sub-app hotkey.",
            );
        }
    }
}

/// Returns and clears a capture the UI has not picked up yet (the window may
/// still have been loading when the event fired).
#[tauri::command]
pub fn take_pending_sub_app_capture() -> Option<SubAppCapture> {
    PENDING_CAPTURE.lock().ok().and_then(|mut slot| slot.take())
}

#[tauri::command]
pub async fn get_sub_apps(app: AppHandle) -> Result<Vec<db::ContextSubApp>, String> {
    let db = db_state(&app);
    run_blocking("get_sub_apps", move || db::query_sub_apps(&db).map_err(|e| e.to_string())).await
}

#[tauri::command]
pub async fn create_sub_app(
    app: AppHandle,
    executable: String,
    app_name: Option<String>,
    label: String,
    icon: Option<String>,
    title_pattern: String,
    match_mode: String,
) -> Result<db::ContextSubApp, String> {
    let db = db_state(&app);
    run_blocking("create_sub_app", move || {
        let match_mode = db::TitleMatchMode::parse(&match_mode).map_err(|e| e.to_string())?;
        db::create_sub_app(
            &db,
            db::NewSubApp {
                executable: &executable,
                app_name: app_name.as_deref(),
                label: &label,
                icon: icon.as_deref(),
                title_pattern: &title_pattern,
                match_mode,
            },
        )
        .map_err(|e| e.to_string())
    })
    .await
}

/// Assigns a sub-app to a Context, or returns it to the list with `null`.
#[tauri::command]
pub async fn assign_sub_app(
    app: AppHandle,
    id: i64,
    context_id: Option<i64>,
) -> Result<db::ContextSubApp, String> {
    let db = db_state(&app);
    run_blocking("assign_sub_app", move || {
        db::assign_sub_app(&db, id, context_id).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn delete_sub_app(app: AppHandle, id: i64) -> Result<(), String> {
    let db = db_state(&app);
    run_blocking("delete_sub_app", move || db::delete_sub_app(&db, id).map_err(|e| e.to_string())).await
}
