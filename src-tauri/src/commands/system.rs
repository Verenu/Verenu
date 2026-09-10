//! Window management, memory, hotkey, autostart, connectivity, dev logs.

use super::*;

// ---------- window management ----------

/// Completes the startup handshake for the window that has just mounted its
/// frontend. This is intentionally reported by the frontend rather than
/// inferred from backend process liveness because WebView2 can display a
/// connection-refused page while the Rust process continues working.
#[tauri::command]
pub fn frontend_ready(
    window: tauri::WebviewWindow,
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
    Ok(())
}

// ---------- local model platform support ----------

/// Lets the frontend show an explanatory notice in place of the local
/// STT/LLM download UI up front, instead of only discovering it's blocked
/// after the user clicks Download and gets an error toast. See
/// `system::platform::is_macos_intel` for the reasoning.
#[tauri::command]
pub async fn local_models_supported_on_this_platform() -> bool {
    // Android has no ONNX Runtime speech builds nor a llama-server runtime
    // (see crate::android::local_ai_supported_on_android and docs/ANDROID.md).
    if cfg!(target_os = "android") {
        return false;
    }
    !crate::system::platform::is_macos_intel()
}

// ---------- memory ----------

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

/// One detected GPU's VRAM. NVIDIA-only (see `memory::gpu_vram_statuses`);
/// absent entirely on other vendors, which the frontend treats as "no signal".
#[derive(serde::Serialize)]
pub struct GpuCapability {
    pub vram_total_mb: u64,
    pub vram_used_mb: u64,
}

/// System hardware snapshot used by the Models tab to recommend presets. RAM
/// is the primary signal (correct for Apple Silicon unified memory too); VRAM
/// is a bonus that's only present on NVIDIA machines. A `total_ram_mb` of 0
/// means the read failed — the frontend must treat that as "unknown, assume
/// capable" rather than "no memory".
#[derive(serde::Serialize)]
pub struct HardwareCapabilities {
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
            total_ram_mb: mem.map(|m| m.total_mb).unwrap_or(0),
            free_ram_mb: mem.map(|m| m.available_mb).unwrap_or(0),
            gpus,
        })
    })
    .await
    .unwrap_or_else(|e| {
        log::error!("{e}");
        HardwareCapabilities {
            total_ram_mb: 0,
            free_ram_mb: 0,
            gpus: Vec::new(),
        }
    })
}

// ---------- developer diagnostics ----------

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
            snapshot.runtime.cleanup_cache = db::cleanup_cache_count(&db)
                .ok()
                .map(|entry_count| serde_json::json!({ "entry_count": entry_count }));
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

/// Reads only the recording session's atomics and the same configured gate
/// threshold used by the pipeline. The lock is held for one short snapshot;
/// no audio buffers, transcripts, or PCM are retained or copied.
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
        .unwrap_or(store::DEFAULT_MIC_GAIN);
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

/// Collects the expensive resource signals only when the diagnostics view or
/// an explicit recording has enabled the profiler. The result intentionally
/// uses `None` for unsupported platform fields instead of presenting a fake
/// zero as a healthy measurement.
fn collect_resource_sample_if_enabled() {
    if !crate::system::diagnostics::should_sample_resources() {
        return;
    }
    let started = std::time::Instant::now();
    let memory_mb = crate::system::memory::measure();
    let mut sample = crate::system::diagnostics::ResourceSnapshot {
        observed_at_ms: 0,
        resident_bytes: (memory_mb > 0).then_some(memory_mb.saturating_mul(1024 * 1024)),
        uptime_ms: Some(diagnostics_uptime_ms()),
        ..Default::default()
    };

    if let Some(gpu) = crate::system::memory::gpu_vram_statuses()
        .into_iter()
        .find(|gpu| gpu.total_mb > 0)
    {
        sample.gpu_memory_bytes = Some(gpu.used_mb.saturating_mul(1024 * 1024));
    }

    #[cfg(target_os = "windows")]
    collect_windows_process_metrics(&mut sample);

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

        // Aggregate process CPU deltas over the bounded tree. The previous
        // sample is replaced wholesale, so terminated/replaced child PIDs
        // cannot accumulate in diagnostics state.
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
                    // GetProcessTimes ticks are per-core (100% = one full core saturated),
                    // but the UI reads "CPU %" as "share of the whole machine", same as Task
                    // Manager. Dividing by the logical core count converts one convention to
                    // the other; without it, a handful of light WebView2 helper processes on
                    // a many-core machine summed to a misleadingly large percentage.
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

/// Keeps the text export useful in a terminal or bug report while retaining
/// the JSON bundle for machine processing. Every source field is already
/// metadata-first/redacted before it reaches this formatter.
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

// ---------- hotkey ----------

#[tauri::command]
pub async fn check_hotkey(key1: String, key2: String) -> Result<bool, String> {
    Ok(crate::core::hotkey::is_hotkey_available(&key1, &key2))
}

#[tauri::command]
pub async fn save_hotkey(app: AppHandle, key1: String, key2: String) -> Result<(), String> {
    let vk1 = crate::core::hotkey::map_code_to_vk(&key1);
    let vk2 = crate::core::hotkey::map_code_to_vk(&key2);
    if vk1 == 0 {
        return Err(format!("Unrecognized key code: {key1}"));
    }
    // An empty second slot is allowed (a single-key hotkey, e.g. macOS F5);
    // only reject a non-empty key code that we can't recognise.
    if !key2.is_empty() && vk2 == 0 {
        return Err(format!("Unrecognized key code: {key2}"));
    }
    crate::core::hotkey::update_keys(vk1, vk2);
    let settings = store::settings_handle(&app)?;
    run_blocking("save_hotkey", move || {
        settings.save_value(store::HOTKEY, serde_json::json!([key1, key2]))
    })
    .await
}

// ---------- autostart ----------

#[tauri::command]
pub async fn set_autostart(_app: AppHandle, enabled: bool) -> Result<(), String> {
    // Registry/file/process operations below are all blocking I/O - run them
    // off the async executor so a slow disk or registry call can't stall
    // other Tokio tasks (audio capture, hotkey handling, etc.).
    #[cfg(target_os = "windows")]
    {
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            use std::os::windows::ffi::OsStrExt;
            use windows::core::PCWSTR;
            use windows::Win32::System::Registry::{
                RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY,
                HKEY_CURRENT_USER, KEY_WRITE, REG_SZ,
            };

            // Quote the path: a Run value is a command line, so an unquoted path
            // containing spaces (e.g. "C:\Program Files\Verenu\verenu.exe") would be
            // parsed as multiple arguments and fail to launch.
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

                if result.is_err() {
                    return Err("Failed to set registry value".to_string());
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())??;
    }

    // macOS: write/remove a LaunchAgent plist that launches the app at login.
    #[cfg(target_os = "macos")]
    {
        let app_handle = _app.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let label = "com.verenu.app";
            let domain = format!("gui/{}", unsafe { libc::getuid() });
            let service_target = format!("{domain}/{label}");
            let home = app_handle
                .path()
                .home_dir()
                .map_err(|e| format!("Failed to get home directory: {e}"))?;
            let dir = home.join("Library/LaunchAgents");
            let plist_path = dir.join(format!("{label}.plist"));

            if enabled {
                let app_path = std::env::current_exe()
                    .map_err(|e| format!("Failed to get executable path: {e}"))?
                    .to_string_lossy()
                    .to_string();
                let mut use_open = false;
                let mut target_path = app_path.clone();
                if let Some(index) = app_path.find(".app/Contents/MacOS/") {
                    target_path = app_path[..index + 4].to_string();
                    use_open = true;
                }

                let escaped_target_path = target_path
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let plist = if use_open {
                    format!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                         <plist version=\"1.0\">\n\
                         <dict>\n\
                           <key>Label</key><string>{label}</string>\n\
                           <key>ProgramArguments</key><array><string>open</string><string>-g</string><string>{escaped_target_path}</string></array>\n\
                           <key>RunAtLoad</key><true/>\n\
                         </dict>\n\
                         </plist>\n"
                    )
                } else {
                    format!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                         <plist version=\"1.0\">\n\
                         <dict>\n\
                           <key>Label</key><string>{label}</string>\n\
                           <key>ProgramArguments</key><array><string>{escaped_target_path}</string></array>\n\
                           <key>RunAtLoad</key><true/>\n\
                         </dict>\n\
                         </plist>\n"
                    )
                };
                std::fs::write(&plist_path, plist).map_err(|e| e.to_string())?;
                let _ = launchctl_bootout(&service_target);
                launchctl_bootstrap(&domain, &plist_path)?;
            } else {
                let _ = launchctl_bootout(&service_target);
                if plist_path.exists() {
                    std::fs::remove_file(&plist_path).map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())??;
    }

    let settings = store::settings_handle(&_app)?;
    run_blocking("set_autostart", move || {
        settings.save_value(store::AUTOSTART_ENABLED, serde_json::json!(enabled))
    })
    .await
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

    // Deliberately no `{:?}` of args: the service target embeds the local
    // plist path. stderr/stdout keep the diagnostic value for the user.
    Err(format!("launchctl failed: {detail}"))
}

// ---------- connectivity ----------

#[tauri::command]
pub async fn check_connectivity() -> bool {
    // Prefer the OS's own network state (see system/connectivity.rs) — on
    // Windows this is a local COM call with zero network traffic; on macOS
    // it's a local routing-table check. Only short-circuit on a confirmed
    // "online" (Some(true)): both NCSI and SCNetworkReachability can report
    // false negatives behind certain VPNs/enterprise proxies, so a "false" or
    // unavailable native result still falls through to the HTTP probe rather
    // than risking a wrong "no internet" banner.
    if let Some(true) = native_connectivity_check().await {
        return true;
    }

    // Probe github.com (a host Verenu already contacts for release downloads)
    // rather than a third-party beacon like google.com, so the connectivity check
    // doesn't quietly phone a separate domain. Deliberately NOT api.github.com:
    // at a 60s poll that would consume the entire 60/hr unauthenticated GitHub
    // API budget and starve the updater's release checks with 403s. Reuses the
    // shared client for connection pooling; GitHub requires a User-Agent.
    crate::api::client::get()
        .head("https://github.com")
        .header("User-Agent", "verenu")
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .is_ok()
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

// ---------- developer logs ----------

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

// ---------- system notifications ----------

#[tauri::command]
pub fn notify_update_available(app: AppHandle, version: String) -> Result<(), String> {
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
