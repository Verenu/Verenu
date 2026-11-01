//! Nonblocking, metadata-only persistence beside the in-memory diagnostics.
use crate::data::db::{self, Db};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, OnceLock,
};
use std::time::Duration;

static SENDER: OnceLock<mpsc::SyncSender<(String, String)>> = OnceLock::new();
static DROPPED: AtomicU64 = AtomicU64::new(0);

pub fn start(
    db: Db,
    #[cfg(target_os = "linux")] gate: std::sync::Arc<crate::linux_activity::ActivityGate>,
) -> Option<std::thread::JoinHandle<()>> {
    let (tx, rx) = mpsc::sync_channel(512);
    if SENDER.set(tx).is_err() {
        return None;
    }
    Some(std::thread::spawn(move || {
        {
            #[cfg(target_os = "linux")]
            let _activity = gate.enter_blocking();
            let _ = db::diagnostic_retention(&db, None);
        }
        let mut maintenance = std::time::Instant::now();
        loop {
            let event = rx.recv_timeout(Duration::from_secs(60));
            #[cfg(target_os = "linux")]
            let _activity = gate.enter_blocking();
            match event {
                Ok((subsystem, fingerprint)) => {
                    if db::record_diagnostic_failure(&db, &subsystem, &fingerprint).is_err() {
                        DROPPED.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if maintenance.elapsed() >= Duration::from_secs(900) {
                if db::diagnostic_retention(&db, None).is_err() {
                    DROPPED.fetch_add(1, Ordering::Relaxed);
                }
                maintenance = std::time::Instant::now();
            }
        }
    }))
}

pub fn dropped() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

pub fn record(event: &super::diagnostics::FailureEvent) {
    let Some(sender) = SENDER.get() else {
        return;
    };
    // Only fixed categories leave memory. No caller-controlled label, cause,
    // model name, prompt, path, transcript, or credential is persisted.
    let subsystem = if event.subsystem.contains("cleanup") {
        "cleanup"
    } else if event.subsystem.contains("transcri") {
        "transcription"
    } else if event.subsystem.contains("pipeline") {
        "pipeline"
    } else if event.subsystem.contains("audio") {
        "audio"
    } else if event.subsystem == "tauri" {
        "command"
    } else {
        "other"
    };
    let category = failure_category(&event.cause);
    if sender
        .try_send((format!("{subsystem}/{category}"), event.fingerprint.clone()))
        .is_err()
    {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

// Coarse investigation labels, not diagnoses. Only these fixed strings are
// persisted; arbitrary provider errors and user content remain in memory.
fn failure_category(cause: &str) -> &'static str {
    let cause = cause.to_ascii_lowercase();
    if cause.contains("429") || cause.contains("rate limit") {
        "rate-limit"
    } else if cause.contains("401") || cause.contains("403") || cause.contains("unauthorized") {
        "authorization"
    } else if cause.contains("timeout") || cause.contains("timed out") {
        "timeout"
    } else if cause.contains("connection") || cause.contains("dns") || cause.contains("network") {
        "connection"
    } else if cause.contains("parse") || cause.contains("invalid json") {
        "response-format"
    } else {
        "unclassified"
    }
}

#[cfg(test)]
mod tests {
    use super::failure_category;

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "uses the process-global diagnostics journal"]
    fn journal_defers_database_writes_until_restart_pause_ends() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        let db = crate::data::db::open(":memory:").unwrap();
        db.lock().unwrap().execute("UPDATE diagnostic_policy SET retention_days = 7 WHERE id = 1", []).unwrap();
        let gate = Arc::new(crate::linux_activity::ActivityGate::default());
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        let worker = super::start(db.clone(), gate).unwrap();
        super::record(&crate::system::diagnostics::FailureEvent {
            id: "synthetic".into(), timestamp_ms: 0, subsystem: "audio".into(),
            operation: None, stage: None, cause: "timeout".into(), duration_ms: None,
            provider: None, model: None, trace_id: None, session_id: None,
            fingerprint: "synthetic".into(), error_category: None, retry_number: None,
            fallback_number: None, measurements: Default::default(), flags: Default::default(),
        });
        std::thread::sleep(Duration::from_millis(100));
        let count = || db.lock().unwrap().query_row("SELECT COUNT(*) FROM diagnostic_daily_failures", [], |row| row.get::<_, i64>(0)).unwrap();
        assert_eq!(count(), 0);
        assert!(!worker.is_finished());
        drop(pause);
        let deadline = Instant::now() + Duration::from_secs(5);
        while count() == 0 {
            assert!(Instant::now() < deadline, "diagnostic event was not written after resuming");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(count(), 1);
    }

    #[test]
    fn journal_categories_are_fixed_labels_never_error_contents() {
        assert_eq!(failure_category("Provider returned 429"), "rate-limit");
        assert_eq!(failure_category("Request timed out"), "timeout");
        assert_eq!(
            failure_category("Public synthetic arbitrary content"),
            "unclassified"
        );
    }
}
