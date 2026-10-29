use super::*;

#[tauri::command]
pub fn frontend_ready(
    window: tauri::WebviewWindow,
    app: AppHandle,
    readiness: tauri::State<'_, crate::FrontendReadiness>,
) -> Result<(), String> {
    match window.label() {
        "main" => readiness
            .main
            .store(true, std::sync::atomic::Ordering::Release),
        "pill" => readiness
            .pill
            .store(true, std::sync::atomic::Ordering::Release),
        label => return Err(format!("Unknown frontend window: {label}")),
    }
    if window.label() == "pill" {
        crate::pipeline::replay_pill_state(&app);
    }
    Ok(())
}

#[tauri::command]
pub async fn local_models_supported_on_this_platform() -> bool {
    if cfg!(target_os = "android") {
        return crate::android::local_ai_supported_on_android();
    }
    !crate::system::platform::is_macos_intel()
}

#[tauri::command]
pub async fn get_memory_mb() -> u64 {
    match run_blocking("get_memory_mb", || Ok(crate::system::memory::measure())).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("{e}");
            0
        }
    }
}

#[derive(serde::Serialize)]
pub struct GpuCapability {
    pub vram_total_mb: u64,
    pub vram_used_mb: u64,
}

#[derive(serde::Serialize)]
pub struct HardwareCapabilities {
    pub is_android: bool,
    pub total_ram_mb: u64,
    pub free_ram_mb: u64,
    pub gpus: Vec<GpuCapability>,
}

#[tauri::command]
pub async fn get_hardware_capabilities() -> HardwareCapabilities {
    run_blocking("get_hardware_capabilities", || {
        let mem = crate::system::memory::system_memory_status();
        let gpus = crate::system::memory::gpu_vram_statuses()
            .into_iter()
            .map(|gpu| GpuCapability {
                vram_total_mb: gpu.total_mb,
                vram_used_mb: gpu.used_mb,
            })
            .collect();
        Ok(HardwareCapabilities {
            is_android: cfg!(target_os = "android"),
            total_ram_mb: mem.map(|m| m.total_mb).unwrap_or(0),
            free_ram_mb: mem.map(|m| m.available_mb).unwrap_or(0),
            gpus,
        })
    })
    .await
    .unwrap_or_else(|e| {
        log::error!("{e}");
        HardwareCapabilities {
            is_android: cfg!(target_os = "android"),
            total_ram_mb: 0,
            free_ram_mb: 0,
            gpus: Vec::new(),
        }
    })
}

#[tauri::command]
pub fn set_diagnostics_monitoring(enabled: bool) {
    crate::system::diagnostics::set_profiler_enabled(enabled);
}

#[tauri::command]
pub fn set_diagnostics_profiling(enabled: bool) {
    crate::system::diagnostics::set_profiling_recording(enabled);
}

#[tauri::command]
pub fn clear_diagnostics() {
    crate::system::diagnostics::reset();
}

#[tauri::command]
pub async fn get_diagnostics_snapshot(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<crate::system::diagnostics::DiagnosticsSnapshot, String> {
    let state = state.inner().clone();
    run_blocking("get_diagnostics_snapshot", move || {
        collect_resource_sample_if_enabled();
        let mut snapshot = crate::system::diagnostics::snapshot_for_ui();
        snapshot.runtime.audio = recording_audio_diagnostics(&app, &state);
        #[cfg(target_os = "linux")]
        {
            snapshot.runtime.window_chrome =
                serde_json::to_value(crate::system::linux_titlebar::capabilities()).ok();
        }
        if let Some(manager) = app.try_state::<crate::local_stt::LocalTranscriptionManager>() {
            snapshot.runtime.local_stt = serde_json::to_value(manager.state()).ok();
        }
        if let Some(manager) = app.try_state::<crate::local_llm::LocalLlmManager>() {
            snapshot.runtime.local_llm = serde_json::to_value(manager.state()).ok();
        }
        if let Some(db) = app.try_state::<DbHandle>() {
            let db = db.inner().clone();
            snapshot.runtime.auto_learn = db::get_auto_learn_status_summary(&db)
                .ok()
                .and_then(|summary| serde_json::to_value(summary).ok());
            snapshot.runtime.cleanup_cache = db::cleanup_cache_count(&db).ok().map(|entry_count| {
                serde_json::json!({ "entry_count": entry_count,
                    "session": crate::pipeline::cache::metrics() })
            });
            snapshot.runtime.sync = db.lock().ok().and_then(|conn| {
                let log_entries = conn
                    .query_row("SELECT COUNT(*) FROM sync_log", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .ok()?;
                let peers = conn
                    .query_row("SELECT COUNT(*) FROM sync_peers", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .ok()?;
                Some(serde_json::json!({ "log_entries": log_entries, "peer_count": peers }))
            });
        }
        Ok(snapshot)
    })
    .await
}

fn recording_audio_diagnostics(app: &AppHandle, state: &SharedState) -> Option<serde_json::Value> {
    use std::sync::atomic::Ordering;

    let sensitivity = pipeline::current_sensitivity_level(state);
    let guard = state.lock().ok()?;
    let session = match &guard.lifecycle {
        pipeline::DictationLifecycle::Recording { session, .. } => session,
        _ => return None,
    };
    let active = session.active.load(Ordering::Acquire);
    let speech_detected = session.speech_detected.load(Ordering::Acquire);
    let stream_error = session.stream_error.load(Ordering::Acquire);
    let raw_rms = f32::from_bits(session.raw_level.load(Ordering::Relaxed));
    let processed_level = f32::from_bits(session.level.load(Ordering::Relaxed));
    drop(guard);

    let gain = store::settings_snapshot(app)
        .ok()
        .map(|settings| store::load_audio_config(&settings).mic_gain)
        .unwrap_or(store::default_mic_gain());
    let raw_rms = raw_rms.is_finite().then_some(raw_rms.max(0.0));
    let processed_level = processed_level
        .is_finite()
        .then_some(processed_level.max(0.0));
    let threshold = pipeline::diagnostic_recording_gate_rms(gain, sensitivity);
    Some(serde_json::json!({
        "active": active,
        "raw_rms": raw_rms,
        "processed_level": processed_level,
        "gate_rms": threshold,
        "would_pass_gate": raw_rms.is_some_and(|rms| rms >= threshold),
        "speech_detected": speech_detected,
        "stream_error": stream_error,
        "adaptive_sensitivity": sensitivity,
        "microphone_gain": gain
    }))
}

fn collect_resource_sample_if_enabled() {
    if !crate::system::diagnostics::should_sample_resources() {
        return;
    }
    let started = std::time::Instant::now();
    let mut sample = crate::system::diagnostics::ResourceSnapshot {
        observed_at_ms: 0,
        uptime_ms: Some(diagnostics_uptime_ms()),
        ..Default::default()
    };
    #[cfg(not(target_os = "linux"))]
    {
        let memory_mb = crate::system::memory::measure();
        sample.resident_bytes = (memory_mb > 0).then_some(memory_mb.saturating_mul(1024 * 1024));
    }

    if let Some(gpu) = crate::system::memory::gpu_vram_statuses()
        .into_iter()
        .find(|gpu| gpu.total_mb > 0)
    {
        sample.gpu_memory_bytes = Some(gpu.used_mb.saturating_mul(1024 * 1024));
        sample.gpu_total_memory_bytes = Some(gpu.total_mb.saturating_mul(1024 * 1024));
    }

    #[cfg(target_os = "windows")]
    collect_windows_process_metrics(&mut sample);
    #[cfg(target_os = "linux")]
    crate::system::linux_proc::fill_resource_snapshot(&mut sample, std::process::id());

    sample.collector_duration_us = Some(started.elapsed().as_micros().min(u64::MAX as u128) as u64);
    crate::system::diagnostics::record_resource_sample(sample);
}

fn diagnostics_uptime_ms() -> u64 {
    use std::sync::OnceLock;
    static STARTED: OnceLock<std::time::Instant> = OnceLock::new();
    STARTED
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(target_os = "windows")]
fn collect_windows_process_metrics(sample: &mut crate::system::diagnostics::ResourceSnapshot) {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessIoCounters, GetProcessTimes, OpenProcess, IO_COUNTERS,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    type PreviousCpuSample = Option<(std::time::Instant, HashMap<u32, u64>)>;
    static PREVIOUS_CPU: OnceLock<Mutex<PreviousCpuSample>> = OnceLock::new();
    static PREVIOUS_IO: OnceLock<Mutex<Option<(std::time::Instant, u64, u64)>>> = OnceLock::new();
    static LOGICAL_CPU_COUNT: OnceLock<u64> = OnceLock::new();

    fn filetime_ticks(value: FILETIME) -> u64 {
        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
    }

    unsafe {
        let process = GetCurrentProcess();
        let mut counters = IO_COUNTERS::default();
        if GetProcessIoCounters(process, &mut counters).is_ok() {
            sample.read_bytes_total = Some(counters.ReadTransferCount);
            sample.write_bytes_total = Some(counters.WriteTransferCount);
            let now = std::time::Instant::now();
            if let Ok(mut previous) = PREVIOUS_IO.get_or_init(|| Mutex::new(None)).lock() {
                if let Some((at, old_read, old_write)) =
                    previous.replace((now, counters.ReadTransferCount, counters.WriteTransferCount))
                {
                    let seconds = now.duration_since(at).as_secs_f64();
                    if seconds > 0.0 {
                        sample.read_bytes_per_sec = Some(
                            counters.ReadTransferCount.saturating_sub(old_read) as f64 / seconds,
                        );
                        sample.write_bytes_per_sec = Some(
                            counters.WriteTransferCount.saturating_sub(old_write) as f64 / seconds,
                        );
                    }
                }
            }
        }

        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut processes = Vec::with_capacity(128);
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name_end = entry
                    .szExeFile
                    .iter()
                    .position(|value| *value == 0)
                    .unwrap_or(entry.szExeFile.len());
                processes.push((
                    entry.th32ProcessID,
                    entry.th32ParentProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..name_end]),
                    entry.cntThreads,
                ));
                if processes.len() >= 512 || Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
        let root = std::process::id();
        let mut parents = vec![root];
        let mut descendants = Vec::new();
        while let Some(parent) = parents.pop() {
            for (pid, ppid, name, threads) in &processes {
                if *ppid == parent
                    && *pid != root
                    && !descendants
                        .iter()
                        .any(|item: &(u32, String, u32)| item.0 == *pid)
                {
                    descendants.push((*pid, name.clone(), *threads));
                    if descendants.len() < 16 {
                        parents.push(*pid);
                    }
                }
            }
            if descendants.len() >= 16 {
                break;
            }
        }
        sample.process_count = Some((descendants.len() + 1) as u32);
        sample.thread_count = Some(
            processes
                .iter()
                .filter(|(pid, _, _, _)| {
                    *pid == root || descendants.iter().any(|item| item.0 == *pid)
                })
                .map(|(_, _, _, threads)| *threads)
                .sum::<u32>(),
        );
        sample.child_processes = descendants
            .into_iter()
            .map(
                |(pid, name, _)| crate::system::diagnostics::ChildProcessSnapshot {
                    label: name,
                    pid: Some(pid),
                    ..Default::default()
                },
            )
            .collect();

        let mut current_cpu = HashMap::with_capacity(sample.child_processes.len() + 1);
        if let Some(ticks) = process_cpu_ticks(process, filetime_ticks) {
            current_cpu.insert(root, ticks);
        }
        for child in &sample.child_processes {
            let Some(pid) = child.pid else { continue };
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                continue;
            };
            if let Some(ticks) = process_cpu_ticks(handle, filetime_ticks) {
                current_cpu.insert(pid, ticks);
            }
            let _ = CloseHandle(handle);
        }
        let now = std::time::Instant::now();
        if let Ok(mut previous) = PREVIOUS_CPU.get_or_init(|| Mutex::new(None)).lock() {
            if let Some((at, old_cpu)) = previous.as_ref() {
                let wall_ticks = now.duration_since(*at).as_secs_f64() * 10_000_000.0;
                if wall_ticks > 0.0 {
                    let cpu_ticks = current_cpu
                        .iter()
                        .map(|(pid, ticks)| ticks.saturating_sub(*old_cpu.get(pid).unwrap_or(&0)))
                        .sum::<u64>();
                    let logical_cpus = *LOGICAL_CPU_COUNT.get_or_init(|| {
                        std::thread::available_parallelism()
                            .map(|n| n.get() as u64)
                            .unwrap_or(1)
                    });
                    sample.cpu_percent = Some(
                        ((cpu_ticks as f64 / wall_ticks) * 100.0 / logical_cpus as f64)
                            .clamp(0.0, 100.0),
                    );
                }
            }
            *previous = Some((now, current_cpu));
        }
    }

    unsafe fn process_cpu_ticks(
        process: windows::Win32::Foundation::HANDLE,
        filetime_ticks: fn(FILETIME) -> u64,
    ) -> Option<u64> {
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user)
            .ok()
            .map(|_| filetime_ticks(kernel).saturating_add(filetime_ticks(user)))
    }
}

#[tauri::command]
pub async fn download_diagnostics_bundle(
    app: AppHandle,
    format: Option<String>,
) -> Result<String, String> {
    run_blocking("download_diagnostics_bundle", move || {
        let downloads = app
            .path()
            .download_dir()
            .map_err(|e| format!("Failed to resolve Downloads directory: {e}"))?;
        std::fs::create_dir_all(&downloads)
            .map_err(|e| format!("Failed to create Downloads path: {e}"))?;
        collect_resource_sample_if_enabled();
        let snapshot = crate::system::diagnostics::snapshot();
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| format!("Failed to serialize diagnostics: {e}"))?;
        let (extension, payload) = if format.as_deref() == Some("text") {
            ("txt", human_diagnostics_bundle(&snapshot))
        } else {
            ("json", json)
        };
        let path = downloads.join(format!("verenu-diagnostics-{timestamp}.{extension}"));
        std::fs::write(&path, payload)
            .map_err(|e| format!("Failed to write diagnostics bundle: {e}"))?;
        Ok(path.display().to_string())
    })
    .await
}

fn human_diagnostics_bundle(snapshot: &crate::system::diagnostics::DiagnosticsSnapshot) -> String {
    use std::fmt::Write;

    let mut output = crate::system::logger::export_header();
    let resource = snapshot.current_resource.as_ref();
    let _ = writeln!(
        output,
        "\n\n[overview]\nprofiler_enabled={} recording={} active_traces={} failures={} dropped_logs={}\nresource.cpu_percent={} resource.resident_bytes={} resource.process_count={} resource.read_bytes_per_sec={} resource.write_bytes_per_sec={}",
        snapshot.profiler_enabled,
        snapshot.profiling_recording,
        snapshot.health.active_trace_count,
        snapshot.health.retained_failure_count,
        snapshot.health.dropped_logs,
        resource.and_then(|item| item.cpu_percent.map(|value| value.to_string())).as_deref().unwrap_or("Unavailable"),
        resource.and_then(|item| item.resident_bytes.map(|value| value.to_string())).as_deref().unwrap_or("Unavailable"),
        resource.and_then(|item| item.process_count.map(|value| value.to_string())).as_deref().unwrap_or("Unavailable"),
        resource.and_then(|item| item.read_bytes_per_sec.map(|value| value.to_string())).as_deref().unwrap_or("Unavailable"),
        resource.and_then(|item| item.write_bytes_per_sec.map(|value| value.to_string())).as_deref().unwrap_or("Unavailable"),
    );
    let _ = writeln!(output, "\n[failures]");
    for failure in &snapshot.latest_failures {
        let _ = writeln!(
            output,
            "{} {} {} cause={} fingerprint={} trace={}",
            failure.timestamp_ms,
            failure.subsystem,
            failure.operation.as_deref().unwrap_or("—"),
            failure.cause,
            failure.fingerprint,
            failure.trace_id.as_deref().unwrap_or("—"),
        );
    }
    let _ = writeln!(output, "\n[pipelines]");
    for trace in snapshot
        .active_pipelines
        .iter()
        .chain(snapshot.recent_pipelines.iter())
    {
        let _ = writeln!(
            output,
            "{} outcome={:?} duration_ms={} spans={}",
            trace.trace_id,
            trace.outcome,
            trace
                .duration_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "Unavailable".to_owned()),
            trace.spans.len(),
        );
    }
    let _ = writeln!(output, "\n[operations]");
    for operation in &snapshot.operations {
        let _ = writeln!(
            output,
            "{} calls={} failures={} avg_ms={} p95_ms={} max_ms={} active={}",
            operation.operation,
            operation.calls,
            operation.failure_count,
            operation
                .average_duration_ms
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "Unavailable".to_owned()),
            operation
                .p95_duration_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "Unavailable".to_owned()),
            operation
                .max_duration_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "Unavailable".to_owned()),
            operation.currently_running,
        );
    }
    let _ = writeln!(output, "\n[structured_logs]");
    for entry in &snapshot.logs {
        let _ = writeln!(
            output,
            "{} {:<5?} {:<24} {}{}",
            entry.timestamp_ms,
            entry.level,
            entry.subsystem,
            entry.message,
            entry
                .trace_id
                .as_deref()
                .map(|trace| format!(" trace={trace}"))
                .unwrap_or_default(),
        );
    }
    output
}

#[tauri::command]
pub async fn set_hotkey_capture(active: bool) -> Result<(), String> {
    if crate::is_dev_session() {
        return Ok(());
    }
    run_blocking("set_hotkey_capture", move || {
        crate::core::hotkey::set_capture_active(active)
    })
    .await
}

#[tauri::command]
pub async fn check_hotkey(app: AppHandle, keys: Vec<String>) -> Result<bool, String> {
    let keys = crate::core::hotkey::normalize_codes(&keys)?;
    let capture = store::settings_handle(&app)?
        .get(store::SUB_APP_CAPTURE_HOTKEY)
        .and_then(|value| {
            value
                .as_str()
                .and_then(crate::core::hotkey::chord::Chord::parse)
        })
        .unwrap_or_else(crate::core::hotkey::chord::Chord::default_for_platform);
    if crate::core::hotkey::shortcut_conflicts(&keys, capture, cfg!(windows))
        || crate::core::hotkey::copy_last_conflicts_with_keys(&keys)
    {
        return Ok(false);
    }
    if crate::is_dev_session() {
        return crate::core::hotkey::validate_hotkey(&keys);
    }
    crate::core::hotkey::is_hotkey_available(&keys)
}

pub(super) static HOTKEY_SETTINGS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn finish_locked<T, F>(lock: &'static tokio::sync::Mutex<()>, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
{
    tokio::spawn(async move {
        let _guard = lock.lock().await;
        work.await
    }).await.map_err(|e| format!("Native settings transaction failed: {e}"))?
}

#[tauri::command]
pub async fn save_hotkey(app: AppHandle, keys: Vec<String>) -> Result<(), String> {
    save_hotkey_transaction(app, keys, None, false).await
}

#[tauri::command]
pub async fn save_hotkey_if_unchanged(app: AppHandle, keys: Vec<String>, expected: serde_json::Value) -> Result<(), String> {
    save_hotkey_transaction(app, keys, Some(expected), false).await
}

pub(crate) async fn save_hotkey_for_agent(app: AppHandle, keys: Vec<String>, expected: serde_json::Value) -> Result<(), String> {
    save_hotkey_transaction(app, keys, Some(expected), true).await
}

async fn save_hotkey_transaction(app: AppHandle, keys: Vec<String>, expected: Option<serde_json::Value>, notify_agent: bool) -> Result<(), String> {
    finish_locked(&HOTKEY_SETTINGS_LOCK, async move {
        if let Some(expected) = expected {
            if store::settings_handle(&app)?.get(store::HOTKEY).unwrap_or(serde_json::Value::Null) != expected {
                return Err("Settings changed while you were editing. Reload them before saving.".into());
            }
        }
        save_hotkey_inner(app.clone(), keys).await?;
        if notify_agent { let _ = app.emit("verenu:agent-settings-changed", vec![store::HOTKEY]); }
        Ok(())
    }).await
}

async fn save_hotkey_inner(app: AppHandle, keys: Vec<String>) -> Result<(), String> {
    let keys = crate::core::hotkey::normalize_codes(&keys)?;
    let ids = crate::core::hotkey::mapped_codes(&keys)?;
    if !check_hotkey(app.clone(), keys.clone()).await? {
        return Err("That shortcut is already assigned. Choose another combination.".into());
    }
    let settings = store::settings_handle(&app)?;
    let previous = settings
        .get(store::HOTKEY)
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                vec!["AltLeft".into(), "Space".into()]
            } else {
                vec!["ControlLeft".into(), "MetaLeft".into()]
            }
        });
    if !crate::is_dev_session() {
        crate::core::hotkey::update_keys(&ids)?;
    }
    let stored_keys = keys.clone();
    run_blocking("save_hotkey", move || {
        if let Err(error) = settings.save_value(store::HOTKEY, serde_json::json!(stored_keys)) {
            if !crate::is_dev_session() {
                if let Ok(ids) = crate::core::hotkey::mapped_codes(&previous) {
                    let _ = crate::core::hotkey::update_keys(&ids);
                }
            }
            return Err(error);
        }
        Ok(())
    })
    .await
}

static AUTOSTART_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    set_autostart_transaction(app, enabled, None, false).await
}

#[tauri::command]
pub async fn set_autostart_if_unchanged(app: AppHandle, enabled: bool, expected: serde_json::Value) -> Result<(), String> {
    set_autostart_transaction(app, enabled, Some(expected), false).await
}

pub(crate) async fn set_autostart_for_agent(app: AppHandle, enabled: bool, expected: serde_json::Value) -> Result<(), String> {
    set_autostart_transaction(app, enabled, Some(expected), true).await
}

async fn set_autostart_transaction(app: AppHandle, enabled: bool, expected: Option<serde_json::Value>, notify_agent: bool) -> Result<(), String> {
    finish_locked(&AUTOSTART_LOCK, async move {
        set_autostart_inner(app.clone(), enabled, expected).await?;
        if notify_agent { let _ = app.emit("verenu:agent-settings-changed", vec![store::AUTOSTART_ENABLED]); }
        Ok(())
    }).await
}

#[tauri::command]
pub async fn initialize_autostart(app: AppHandle) -> Result<(), String> {
    finish_locked(&AUTOSTART_LOCK, async move {
        if store::settings_handle(&app)?.get(store::AUTOSTART_ENABLED).is_some() {
            return Ok(());
        }
        set_autostart_inner(app, true, Some(serde_json::Value::Null)).await
    }).await
}

pub(crate) async fn reconcile_autostart(app: AppHandle) -> Result<(), String> {
    finish_locked(&AUTOSTART_LOCK, async move {
    let enabled = store::settings_handle(&app)?.get(store::AUTOSTART_ENABLED)
        .and_then(|value| value.as_bool());
    if let Some(enabled) = enabled {
        run_blocking("reconcile_autostart", move || {
            #[cfg(target_os = "macos")]
            {
                let path = app.path().home_dir().map_err(|e| e.to_string())?
                    .join("Library/LaunchAgents/com.verenu.app.plist");
                let service = format!("gui/{}/com.verenu.app", unsafe { libc::getuid() });
                let loaded = std::process::Command::new("launchctl")
                    .args(["print", &service]).output()
                    .map_err(|e| format!("Failed to inspect LaunchAgent: {e}"))?.status.success();
                let plist_matches = std::fs::read(&path).ok()
                    .is_some_and(|contents| launch_agent_plist().is_ok_and(|expected| contents == expected.as_bytes()));
                if (enabled && loaded && plist_matches) || (!enabled && !loaded && !path.exists()) {
                    return Ok(());
                }
            }
            set_autostart_native(app, enabled)
        }).await?;
    }
    Ok(())
    }).await
}

async fn set_autostart_inner(app: AppHandle, enabled: bool, expected: Option<serde_json::Value>) -> Result<(), String> {
    let settings = store::settings_handle(&app)?;
    run_blocking("set_autostart", move || {
        commit_autostart(&settings, enabled, expected, || set_autostart_native(app, enabled))
    }).await
}

fn commit_autostart(
    settings: &store::SettingsHandle,
    enabled: bool,
    expected: Option<serde_json::Value>,
    apply_native: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    settings.save_values_with_commit(
        [(store::AUTOSTART_ENABLED, serde_json::json!(enabled))],
        |current| {
            if let Some(expected) = expected {
                if current.get_cloned(store::AUTOSTART_ENABLED).unwrap_or(serde_json::Value::Null) != expected {
                    return Err("Settings changed while you were editing. Reload them before saving.".into());
                }
            }
            Ok(())
        },
        apply_native,
    ).map(|_| ())
}

#[cfg(any(target_os = "macos", test))]
fn restore_after_native_failure(
    result: Result<(), String>,
    rollback_needed: bool,
    rollback: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if !rollback_needed => Err(error),
        Err(error) => match rollback() {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!("{error}; LaunchAgent rollback failed: {rollback}")),
        },
    }
}

#[cfg(target_os = "macos")]
fn launch_agent_plist() -> Result<String, String> {
    let app_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get executable path: {e}"))?
        .to_string_lossy().to_string();
    let arguments = if let Some(index) = app_path.find(".app/Contents/MacOS/") {
        let path = crate::api::cleanup::escape_transcript_xml(&app_path[..index + 4]);
        format!("<string>open</string><string>-g</string><string>{path}</string>")
    } else {
        let path = crate::api::cleanup::escape_transcript_xml(&app_path);
        format!("<string>{path}</string>")
    };
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
           <key>Label</key><string>com.verenu.app</string>\n\
           <key>ProgramArguments</key><array>{arguments}</array>\n\
           <key>RunAtLoad</key><true/>\n\
         </dict>\n\
         </plist>\n"
    ))
}

#[cfg(any(target_os = "macos", test))]
fn write_launch_agent_atomically(path: &std::path::Path, contents: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("LaunchAgent path has no parent")?;
    let temporary = parent.join(format!(".verenu-launchagent-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true)
            .open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(contents).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(temporary); }
    result
}

#[cfg(test)]
mod autostart_commit_tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_request_keeps_lock_until_native_transaction_finishes() {
        static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let first = tokio::spawn(finish_locked(&LOCK, async move {
            let _ = started_tx.send(());
            let _ = release_rx.await;
            Ok(())
        }));
        started_rx.await.unwrap();
        first.abort();
        let (second_tx, mut second_rx) = tokio::sync::oneshot::channel();
        let second = tokio::spawn(finish_locked(&LOCK, async move {
            let _ = second_tx.send(());
            Ok(())
        }));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(30), &mut second_rx).await.is_err());
        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), second_rx).await.unwrap().unwrap();
        second.await.unwrap().unwrap();
    }

    #[test]
    fn persistence_failure_does_not_change_native_autostart() {
        let parent = std::env::temp_dir().join(format!("verenu-autostart-parent-{}", uuid::Uuid::new_v4()));
        std::fs::write(&parent, b"not a directory").unwrap();
        let settings = store::SettingsHandle::empty_for_test(parent.join("settings.json"));
        let mut applied = false;
        assert!(commit_autostart(&settings, true, None, || { applied = true; Ok(()) }).is_err());
        assert!(!applied);
        assert_eq!(settings.get(store::AUTOSTART_ENABLED), None);
        std::fs::remove_file(parent).unwrap();
    }

    #[test]
    fn native_failure_restores_previous_saved_autostart() {
        let path = std::env::temp_dir().join(format!("verenu-autostart-{}.json", uuid::Uuid::new_v4()));
        let settings = store::SettingsHandle::empty_for_test(path.clone());
        settings.save_value(store::AUTOSTART_ENABLED, serde_json::json!(false)).unwrap();
        assert!(commit_autostart(&settings, true, None, || Err("native failure".into())).is_err());
        assert_eq!(settings.get(store::AUTOSTART_ENABLED), Some(serde_json::json!(false)));
        let disk: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(disk[store::AUTOSTART_ENABLED], false);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn stale_autostart_write_does_not_touch_native_state() {
        let path = std::env::temp_dir().join(format!("verenu-autostart-{}.json", uuid::Uuid::new_v4()));
        let settings = store::SettingsHandle::empty_for_test(path.clone());
        settings.save_value(store::AUTOSTART_ENABLED, serde_json::json!(false)).unwrap();
        let mut applied = false;
        assert!(commit_autostart(&settings, true, Some(serde_json::Value::Null), || { applied = true; Ok(()) }).is_err());
        assert!(!applied);
        assert_eq!(settings.get(store::AUTOSTART_ENABLED), Some(serde_json::json!(false)));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn partial_native_failure_restores_old_native_state_and_reports_rollback_failure() {
        let native = std::cell::Cell::new(false);
        let result = restore_after_native_failure(
            { native.set(true); Err("bootstrap failed".into()) },
            true,
            || { native.set(false); Ok(()) },
        );
        assert_eq!(result.unwrap_err(), "bootstrap failed");
        assert!(!native.get());
        let error = restore_after_native_failure(Err("bootstrap failed".into()), true, || Err("restore failed".into())).unwrap_err();
        assert!(error.contains("bootstrap failed") && error.contains("restore failed"));
    }

    #[test]
    fn failure_before_native_changes_does_not_unload_a_working_service() {
        let mut loaded = true;
        let result = restore_after_native_failure(Err("plist write failed".into()), false, || {
            loaded = false;
            Err("rollback failed".into())
        });
        assert_eq!(result.unwrap_err(), "plist write failed");
        assert!(loaded);
    }

    #[test]
    fn launch_agent_replacement_preserves_existing_file_when_write_cannot_start() {
        let dir = std::env::temp_dir().join(format!("verenu-agent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("agent.plist");
        std::fs::write(&path, b"original").unwrap();
        let interrupted = dir.join(".verenu-launchagent-interrupted.tmp");
        std::fs::write(&interrupted, b"partially written").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        write_launch_agent_atomically(&path, b"replacement").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert!(write_launch_agent_atomically(&dir.join("missing/agent.plist"), b"new").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn set_autostart_native(_app: AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
            use std::os::windows::ffi::OsStrExt;
            use windows::core::PCWSTR;
            use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
            use windows::Win32::System::Registry::{
                RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY,
                HKEY_CURRENT_USER, KEY_WRITE, REG_SZ,
            };

            let app_path = format!(
                "\"{}\"",
                std::env::current_exe()
                    .map_err(|e| format!("Failed to get executable path: {e}"))?
                    .to_string_lossy()
            );

            let subkey: Vec<u16> =
                std::ffi::OsStr::new("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
            let value_name: Vec<u16> = std::ffi::OsStr::new("Verenu")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            unsafe {
                let mut hkey = HKEY::default();
                let status = RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    PCWSTR(subkey.as_ptr()),
                    None,
                    KEY_WRITE,
                    std::ptr::addr_of_mut!(hkey),
                );

                if status.is_err() {
                    return Err("Failed to open registry key".to_string());
                }

                let result = if enabled {
                    let app_path_wide: Vec<u16> = std::ffi::OsStr::new(&app_path)
                        .encode_wide()
                        .chain(std::iter::once(0))
                        .collect();
                    RegSetValueExW(
                        hkey,
                        PCWSTR(value_name.as_ptr()),
                        None,
                        REG_SZ,
                        Some(std::slice::from_raw_parts(
                            app_path_wide.as_ptr() as *const u8,
                            app_path_wide.len() * 2,
                        )),
                    )
                } else {
                    RegDeleteValueW(hkey, PCWSTR(value_name.as_ptr()))
                };

                let _ = RegCloseKey(hkey);

                if result.is_err() && (enabled || result != ERROR_FILE_NOT_FOUND) {
                    return Err("Failed to set registry value".to_string());
                }
            }
    }

    #[cfg(target_os = "macos")]
    {
            let app_handle = _app;
            let label = "com.verenu.app";
            let domain = format!("gui/{}", unsafe { libc::getuid() });
            let service_target = format!("{domain}/{label}");
            let home = app_handle
                .path()
                .home_dir()
                .map_err(|e| format!("Failed to get home directory: {e}"))?;
            let dir = home.join("Library/LaunchAgents");
            let plist_path = dir.join(format!("{label}.plist"));
            let previous_plist = match std::fs::read(&plist_path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(format!("Failed to read LaunchAgent: {error}")),
            };
            let was_loaded = std::process::Command::new("launchctl")
                .args(["print", &service_target]).output()
                .map_err(|e| format!("Failed to inspect LaunchAgent: {e}"))?.status.success();
            if was_loaded && previous_plist.is_none() {
                if enabled {
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    return write_launch_agent_atomically(&plist_path, launch_agent_plist()?.as_bytes());
                }
                return Err("Cannot safely change a loaded LaunchAgent whose plist is missing".into());
            }

            let plist_changed = std::cell::Cell::new(false);
            let service_changed = std::cell::Cell::new(false);
            let apply = || -> Result<(), String> { if enabled {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                write_launch_agent_atomically(&plist_path, launch_agent_plist()?.as_bytes())?;
                plist_changed.set(true);
                if was_loaded {
                    launchctl_bootout(&service_target)?;
                    service_changed.set(true);
                }
                service_changed.set(true);
                launchctl_bootstrap(&domain, &plist_path)?;
            } else {
                if was_loaded {
                    launchctl_bootout(&service_target)?;
                    service_changed.set(true);
                }
                if plist_path.exists() {
                    std::fs::remove_file(&plist_path).map_err(|e| e.to_string())?;
                    plist_changed.set(true);
                }
            } Ok(()) };
            let result = apply();
            restore_after_native_failure(result, plist_changed.get() || service_changed.get(), || -> Result<(), String> {
                    if service_changed.get() && std::process::Command::new("launchctl").args(["print", &service_target])
                        .output().map_err(|e| e.to_string())?.status.success() {
                        launchctl_bootout(&service_target)?;
                    }
                    if plist_changed.get() { match previous_plist {
                        Some(bytes) => write_launch_agent_atomically(&plist_path, &bytes)?,
                        None => match std::fs::remove_file(&plist_path) {
                            Ok(()) => (),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                            Err(e) => return Err(e.to_string()),
                        },
                    }}
                    if service_changed.get() && was_loaded { launchctl_bootstrap(&domain, &plist_path)?; }
                    Ok(())
                })?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = enabled;
    Ok(())
}

#[cfg(target_os = "macos")]
fn launchctl_bootstrap(domain: &str, plist_path: &std::path::Path) -> Result<(), String> {
    run_launchctl(&[
        "bootstrap",
        domain,
        plist_path.to_str().ok_or("Invalid plist path")?,
    ])
}

#[cfg(target_os = "macos")]
fn launchctl_bootout(service_target: &str) -> Result<(), String> {
    run_launchctl(&["bootout", service_target])
}

#[cfg(target_os = "macos")]
fn run_launchctl(args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new("launchctl")
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run launchctl: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("exit status {}", output.status)
    };

    Err(format!("launchctl failed: {detail}"))
}

#[tauri::command]
pub async fn check_connectivity() -> bool {
    if let Some(true) = native_connectivity_check().await {
        crate::system::connectivity::note_online();
        return true;
    }

    let online = crate::api::client::get()
        .head("https://github.com")
        .header("User-Agent", "verenu")
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .is_ok();
    if online {
        crate::system::connectivity::note_online();
    }
    online
}

#[cfg(windows)]
async fn native_connectivity_check() -> Option<bool> {
    // COM requires apartment init on the calling thread, so run on a
    // dedicated blocking thread rather than whatever tokio worker polls this.
    tokio::task::spawn_blocking(crate::system::connectivity::check_native)
        .await
        .ok()
        .flatten()
}

#[cfg(not(windows))]
async fn native_connectivity_check() -> Option<bool> {
    crate::system::connectivity::check_native()
}

#[tauri::command]
pub fn get_recent_logs(limit: Option<usize>) -> Vec<String> {
    crate::system::logger::recent(limit)
}

#[tauri::command]
pub fn subscribe_log_stream() {
    crate::system::logger::subscribe_log_stream();
}

#[tauri::command]
pub fn unsubscribe_log_stream() {
    crate::system::logger::unsubscribe_log_stream();
}

#[tauri::command]
pub async fn download_logs(app: AppHandle) -> Result<String, String> {
    run_blocking("download_logs", move || {
        crate::system::logger::export_to_downloads(&app)
    })
    .await
}

#[tauri::command]
pub fn set_dev_logging_enabled(enabled: bool) {
    crate::system::logger::set_verbose(enabled);
}

#[tauri::command]
pub fn get_dev_logging_enabled() -> bool {
    crate::system::logger::is_verbose()
}

#[tauri::command]
pub fn notify_update_available(app: AppHandle, version: String) -> Result<(), String> {
    if let Some(analytics) = app.try_state::<crate::analytics::Analytics>() {
        analytics.updater_event("update_available", None, Some(&version));
    }
    crate::system::notify::notify_update_available(&app, &version)
}

#[tauri::command]
pub fn notify_provider_and_global_message(
    app: AppHandle,
    provider_summary: String,
    global_message: String,
) -> Result<(), String> {
    crate::system::notify::notify_provider_and_global_message(
        &app,
        &provider_summary,
        &global_message,
    )
}

#[tauri::command]
pub fn test_notifications(app: AppHandle, notification_type: Option<String>) -> Result<(), String> {
    crate::system::notify::notify_test_notification(
        &app,
        notification_type.as_deref().unwrap_or("update"),
    )
}
