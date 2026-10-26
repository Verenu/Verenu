//! Nonblocking, metadata-only persistence beside the in-memory diagnostics.
use crate::data::db::{self, Db};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, OnceLock,
};
use std::time::Duration;

static SENDER: OnceLock<mpsc::SyncSender<(String, String)>> = OnceLock::new();
static DROPPED: AtomicU64 = AtomicU64::new(0);

pub fn start(db: Db) {
    let (tx, rx) = mpsc::sync_channel(512);
    if SENDER.set(tx).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let _ = db::diagnostic_retention(&db, None);
        let mut maintenance = std::time::Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_secs(60)) {
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
    });
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
