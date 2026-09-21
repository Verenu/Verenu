use super::*;

/// Frontend setup analytics accepts only a small fixed vocabulary. The
/// transport and identity remain owned by the Rust analytics boundary.
#[tauri::command]
pub fn analytics_setup_event(
    app: AppHandle,
    event: String,
    step: Option<String>,
    duration_ms: Option<u64>,
) -> Result<(), String> {
    let event_name = match event.as_str() {
        "setup_started" => "setup_started",
        "setup_step_viewed" => "setup_step_viewed",
        "setup_step_completed" => "setup_step_completed",
        "setup_step_duration" => "setup_step_duration",
        "setup_completed" => "setup_completed",
        _ => return Err("Unsupported analytics setup event".to_owned()),
    };
    let allowed_step = match step.as_deref() {
        None => None,
        Some("intro") => Some("intro"),
        Some("analytics") => Some("analytics"),
        Some("provider") => Some("provider"),
        Some("api_key") => Some("api_key"),
        Some("permissions") => Some("permissions"),
        Some("models") => Some("models"),
        Some("writing_style") => Some("writing_style"),
        Some("language") => Some("language"),
        Some("audio_environment") => Some("audio_environment"),
        Some("try_it") => Some("try_it"),
        Some("done") => Some("done"),
        Some(_) => return Err("Unsupported analytics setup step".to_owned()),
    };
    if matches!(
        event_name,
        "setup_step_viewed" | "setup_step_completed" | "setup_step_duration"
    ) && allowed_step.is_none()
    {
        return Err("Setup step is required".to_owned());
    }
    if event_name == "setup_step_duration" && duration_ms.is_none() {
        return Err("Setup step duration is required".to_owned());
    }
    #[cfg(desktop)]
    if let Some(analytics) = app.try_state::<crate::analytics::Analytics>() {
        if event_name == "setup_step_duration" {
            analytics
                .setup_step_duration(allowed_step.unwrap_or("unknown"), duration_ms.unwrap_or(0));
        } else {
            analytics.setup_event(event_name, allowed_step);
        }
    }
    Ok(())
}

/// Records only a fixed frontend error family. The WebView never passes an
/// Error object, message, URL, rejected value, or JavaScript stack across IPC.
#[tauri::command]
pub fn analytics_frontend_exception(
    app: AppHandle,
    kind: String,
    handled: bool,
) -> Result<(), String> {
    let code = match kind.as_str() {
        "frontend_unhandled" => "frontend_unhandled",
        "frontend_handled" => "frontend_handled",
        _ => return Err("Unsupported frontend analytics exception".to_owned()),
    };
    #[cfg(desktop)]
    if let Some(analytics) = app.try_state::<crate::analytics::Analytics>() {
        analytics.capture_frontend_exception(code, handled);
    }
    Ok(())
}
