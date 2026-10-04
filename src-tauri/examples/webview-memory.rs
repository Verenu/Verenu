//! Opt-in Linux display/IPC/PSS comparison for independent and related views.
//! No app settings, audio, credentials, hotkeys, or provider calls are used.
//! Run with `-- shared` or `-- separate`; each run exits after its probes.

#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "../src/system/linux_webview.rs"]
mod policy;

#[cfg(target_os = "linux")]
mod probe {
    use super::policy;
    use gtk::glib::object::ObjectType;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tauri::{Emitter, Manager};
    use webkit2gtk::WebViewExt;

    #[derive(Default)]
    struct Reports {
        ready: HashSet<String>,
        events: HashSet<String>,
    }

    #[tauri::command]
    fn memory_probe(
        window: tauri::WebviewWindow,
        reports: tauri::State<'_, Arc<Mutex<Reports>>>,
        expected_label: String,
        stage: String,
    ) -> Result<(), String> {
        if window.label() != expected_label {
            return Err("IPC reached the wrong window".into());
        }
        let mut reports = reports.lock().map_err(|_| "probe lock poisoned")?;
        match stage.as_str() {
            "ready" => {
                reports.ready.insert(expected_label);
            }
            "event" => {
                reports.events.insert(expected_label);
            }
            _ => return Err("unknown probe stage".into()),
        }
        Ok(())
    }

    const HTML: &str = r#"<!doctype html><html><body><p>WebView memory probe</p><script>
    (async () => {
      const api = window.__TAURI_INTERNALS__;
      const label = location.pathname.slice(1);
      await api.invoke('plugin:event|listen', { event: 'memory-probe', target: { kind: 'Any' },
        handler: api.transformCallback(() => api.invoke('memory_probe', {
          expectedLabel: label, stage: 'event'
        }))
      });
      await api.invoke('memory_probe', { expectedLabel: label, stage: 'ready' });
      document.body.dataset.ready = label;
    })();
    </script></body></html>"#;

    fn build_related_window(
        main: &tauri::WebviewWindow,
        build: impl FnOnce(webkit2gtk::WebView) -> tauri::Result<tauri::WebviewWindow> + Send + 'static,
    ) -> tauri::Result<tauri::WebviewWindow> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        main.with_webview(move |platform| {
            let _ = sender.send(build(platform.inner()));
        })?;
        receiver
            .recv()
            .map_err(|_| tauri::Error::FailedToReceiveMessage)?
    }

    fn pss_tree(root: u32) -> (u64, usize) {
        let mut pending = vec![root];
        let mut total_kb = 0;
        let mut renderers = 0;
        while let Some(pid) = pending.pop() {
            let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
            if comm.trim() == "WebKitWebProces" {
                renderers += 1;
            }
            if let Ok(rollup) = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
                let pss_kb = rollup
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Pss:")
                            .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
                    })
                    .unwrap_or(0);
                total_kb += pss_kb;
                println!("process={} pss_kb={pss_kb}", comm.trim());
            }
            if let Ok(children) =
                std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
            {
                pending.extend(
                    children
                        .split_whitespace()
                        .filter_map(|value| value.parse::<u32>().ok()),
                );
            }
        }
        (total_kb, renderers)
    }

    pub fn run() {
        let shared = match std::env::args().nth(1).as_deref() {
            Some("shared") => true,
            Some("separate") => false,
            _ => panic!("expected shared or separate"),
        };
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        let reports = Arc::new(Mutex::new(Reports::default()));
        tauri::Builder::default()
            .manage(reports.clone())
            .invoke_handler(tauri::generate_handler![memory_probe])
            .register_uri_scheme_protocol("memory", |_, _| {
                tauri::http::Response::builder().header("Content-Type", "text/html")
                    .body(HTML.as_bytes().to_vec()).expect("HTML response")
            })
            .setup(move |app| {
                let main = tauri::WebviewWindowBuilder::new(app, "main",
                    tauri::WebviewUrl::External("memory://localhost/main".parse()?))
                    .title("Verenu memory probe main").inner_size(640.0, 400.0).build()?;
                policy::configure_window(&main);
                let handle = app.handle().clone();
                let build = move |related: Option<webkit2gtk::WebView>| {
                    let mut builder = tauri::WebviewWindowBuilder::new(&handle, "pill",
                        tauri::WebviewUrl::External("memory://localhost/pill".parse().unwrap()))
                        .title("Verenu memory probe pill").inner_size(200.0, 200.0)
                        .transparent(true).decorations(false);
                    let main_manager = related.as_ref().map(|view| view.user_content_manager().unwrap().as_ptr() as usize);
                    if let Some(related) = related { builder = builder.with_related_view(related); }
                    let pill = builder.build()?;
                    policy::configure_window(&pill);
                    // Wry must supply a distinct content manager even for related
                    // views, so IPC handlers and scripts remain window-specific.
                    pill.with_webview(move |pill_platform| {
                        assert_ne!(main_manager, Some(pill_platform.inner().user_content_manager().unwrap().as_ptr() as usize));
                    })?;
                    Ok(pill)
                };
                if shared {
                    build_related_window(&main, move |related| build(Some(related)))?;
                } else {
                    build(None)?;
                }
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    for _ in 0..100 {
                        if reports.lock().unwrap().ready.len() == 2 { break; }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    if reports.lock().unwrap().ready.len() != 2 {
                        eprintln!("both IPC handshakes failed"); handle.exit(1); return;
                    }
                    for round in 0..3 {
                        let main = handle.get_webview_window("main").unwrap();
                        if round == 1 { main.hide().unwrap(); }
                        if round == 2 { main.show().unwrap(); }
                        reports.lock().unwrap().events.clear();
                        handle.emit("memory-probe", 1).expect("native event");
                        for _ in 0..50 {
                            if reports.lock().unwrap().events.len() == 2 { break; }
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                        if reports.lock().unwrap().events.len() != 2 {
                            eprintln!("both event callbacks failed in round {round}"); handle.exit(1); return;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let mut samples = Vec::new();
                    let mut renderers = 0;
                    for _ in 0..3 {
                        // Match the production app's idle native allocator trim.
                        #[cfg(target_env = "gnu")]
                        unsafe { libc::malloc_trim(0); }
                        let (pss_kb, count) = pss_tree(std::process::id());
                        samples.push(pss_kb);
                        renderers = count;
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    if renderers != if shared { 1 } else { 2 } {
                        eprintln!("unexpected renderer count {renderers}"); handle.exit(1); return;
                    }
                    samples.sort_unstable();
                    let pss_kb = samples[1];
                    println!("mode={} median_pss_kb={pss_kb} samples_kb={samples:?} renderers={renderers} ipc_windows=2 event_rounds=3 hidden_main=verified",
                        if shared { "shared" } else { "separate" });
                    handle.exit(0);
                });
                Ok(())
            })
            .run(context).expect("display probe");
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    probe::run();
}
