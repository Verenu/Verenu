//! Effective desktop shortcuts, distinct from the user's saved preferences.

use serde::Serialize;
use std::sync::Mutex;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
#[cfg(target_os = "linux")]
use tauri::Emitter;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShortcutStatus {
    pub id: String,
    pub requested: String,
    pub active: Option<String>,
    pub codes: Vec<String>,
    pub note: Option<String>,
}

static STATUS: Mutex<Vec<ShortcutStatus>> = Mutex::new(Vec::new());
#[cfg(target_os = "linux")]
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

#[cfg(target_os = "linux")]
pub fn initialize(app: tauri::AppHandle) {
    let _ = APP.set(app);
}

#[cfg(target_os = "linux")]
pub fn publish(status: ShortcutStatus) {
    let Ok(mut statuses) = STATUS.lock() else {
        return;
    };
    if let Some(existing) = statuses.iter_mut().find(|s| s.id == status.id) {
        if *existing == status {
            return;
        }
        *existing = status;
    } else {
        statuses.push(status);
    }
    if let Some(app) = APP.get() {
        let _ = app.emit("verenu:shortcuts-changed", &*statuses);
    }
}

#[tauri::command]
pub fn get_shortcut_status() -> Vec<ShortcutStatus> {
    STATUS.lock().map(|s| s.clone()).unwrap_or_default()
}
