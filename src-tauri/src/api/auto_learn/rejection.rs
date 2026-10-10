use super::*;
use crate::core::context::ResolvedContextIdentity;
use serde::Serialize;

#[derive(Clone, Serialize)]
struct DictionaryRejectionEvent {
    context_id: i64,
    deleted: usize,
}

#[derive(Debug)]
enum RejectionTarget {
    DictionaryCorrections {
        correction_ids: Vec<i64>,
        context: ResolvedContextIdentity,
    },
    CacheKey {
        key: String,
    },
}

impl RejectionTarget {
    fn monitor_key_prefix(&self) -> &'static str {
        match self {
            RejectionTarget::DictionaryCorrections { .. } => "rejection",
            RejectionTarget::CacheKey { .. } => "cache_rejection",
        }
    }

    fn window_secs(&self) -> u64 {
        match self {
            RejectionTarget::DictionaryCorrections { .. } => REJECTION_WINDOW_SECS,
            RejectionTarget::CacheKey { .. } => CACHE_REJECTION_WINDOW_SECS,
        }
    }

    fn context_id(&self) -> Option<i64> {
        match self {
            RejectionTarget::DictionaryCorrections { context, .. } => Some(context.id),
            RejectionTarget::CacheKey { .. } => None,
        }
    }
}

fn rejection_monitor_key(injected_text: &str, target: &RejectionTarget) -> String {
    // A global active-monitor set must distinguish identical injected text in
    // two resolved Contexts. Cache invalidation has no Context scope, while a
    // dictionary rejection does, so only the latter contributes its stable
    // Context ID to the key.
    let prefix = target.monitor_key_prefix();
    let key_context = target
        .context_id()
        .map(|id| id.to_string())
        .unwrap_or_default();
    let key_material = format!("{prefix}:{key_context}");
    let (text_hash, context_hash) = pair_hash(injected_text, &key_material);
    format!("{prefix}:{context_hash}:{text_hash}")
}

fn apply_rejection(target: &RejectionTarget, db: &DbHandle, app: &AppHandle, prefix: &str) {
    match target {
        RejectionTarget::DictionaryCorrections {
            correction_ids,
            context,
        } => {
            let deleted =
                match db::delete_auto_learned_corrections_by_ids(db, context.id, correction_ids) {
                    Ok(deleted) => deleted,
                    Err(e) => {
                        log::warn!("{prefix}: delete failed: {e}");
                        return;
                    }
                };
            if deleted > 0 {
                app.emit(
                    "verenu:dictionary-entry-rejected",
                    DictionaryRejectionEvent {
                        context_id: context.id,
                        deleted,
                    },
                )
                .ok();
            }
        }
        RejectionTarget::CacheKey { key } => {
            if let Err(e) = db::cleanup_cache_delete_by_key(db, key) {
                log::warn!("{prefix}: delete failed: {e}");
            } else {
                app.emit("verenu:cleanup-cache-invalidated", ()).ok();
            }
        }
    }
}

#[cfg(windows)]
pub(super) fn is_target_window_focused(target_hwnd: usize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe { GetForegroundWindow().0 as usize == target_hwnd }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn is_target_window_focused(target_id: usize) -> bool {
    target_id != 0 && crate::core::window_context::get_foreground_hwnd() == target_id
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn is_target_window_focused(_target_hwnd: usize) -> bool {
    false
}

fn run_rejection_monitor(
    injected_text: String,
    target: RejectionTarget,
    target_hwnd: usize,
    db: DbHandle,
    app: AppHandle,
) {
    let key = rejection_monitor_key(&injected_text, &target);
    let inserted = match active_monitors().lock() {
        Ok(mut active) if active.len() < 32 => active.insert(key.clone()),
        Err(_) => false,
        _ => false,
    };
    if !inserted {
        return;
    }

    let guard = MonitorKeyGuard::new(key);
    // Keep native accessibility identities on one OS thread. In particular,
    // UIA elements belong to the COM apartment that read the baseline.
    let spawned = std::thread::Builder::new()
        .name("auto_learn_rejection".into())
        .spawn(move || {
            let _guard = guard;
            let prefix = target.monitor_key_prefix();
            let read_anchored_baseline = || {
                read_monitor_text(&injected_text, true).and_then(|text| {
                    find_unique_anchor(&text.text, &injected_text).map(|anchor| (text, anchor))
                })
            };

            std::thread::sleep(std::time::Duration::from_millis(BASELINE_CAPTURE_DELAY_MS));
            if !is_target_window_focused(target_hwnd) {
                return;
            }
            let mut baseline = read_anchored_baseline();

            if baseline.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(BASELINE_RETRY_DELAY_MS));
                if !is_target_window_focused(target_hwnd) {
                    return;
                }
                baseline = read_anchored_baseline();
            }

            let Some((baseline, anchor)) = baseline else {
                // Missing text can mean focus moved to another field in the same
                // app. Without an observed insertion there is no proof of deletion.
                log::debug!("{prefix}: no verified baseline, skipping");
                return;
            };

            let rejection_threshold = injected_text.chars().count() / 10;
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(target.window_secs());
            let mut stable_gate = StableTextGate::default();

            loop {
                if std::time::Instant::now() >= deadline {
                    break;
                }

                std::thread::sleep(std::time::Duration::from_millis(REJECTION_POLL_MS));
                if !is_target_window_focused(target_hwnd) {
                    stable_gate = StableTextGate::default();
                    continue;
                }
                let current = match read_monitor_text(&injected_text, false) {
                    Some(text) if baseline.identity.matches(&text.identity) => text.text,
                    _ => {
                        stable_gate = StableTextGate::default();
                        continue;
                    }
                };
                let Some(current) = stable_gate.observe(current) else {
                    continue;
                };

                let rejected = match current_anchored_span(&baseline.text, current, anchor) {
                    Some(span) => span.chars().count() <= rejection_threshold,
                    None => false,
                };

                if rejected {
                    // Guard against false positives from window switches: only fire
                    // if the original injection window is still in the foreground.
                    let still_focused = is_target_window_focused(target_hwnd);
                    if still_focused {
                        log::info!("{prefix}: deletion detected, firing rejection");
                        apply_rejection(&target, &db, &app, prefix);
                        return;
                    }
                    log::debug!("{prefix}: rejection signal but window switched, ignoring");
                }
            }
            log::debug!("{prefix}: window expired, no rejection detected");
        });
    if let Err(error) = spawned {
        log::warn!("auto-learn rejection thread failed: {error}");
    }
}

pub fn start_rejection_monitor(
    injected_text: String,
    applied_correction_ids: Vec<i64>,
    target_hwnd: usize,
    context: ResolvedContextIdentity,
    db: DbHandle,
    app: AppHandle,
) {
    if applied_correction_ids.is_empty() {
        return;
    }
    run_rejection_monitor(
        injected_text,
        RejectionTarget::DictionaryCorrections {
            correction_ids: applied_correction_ids,
            context,
        },
        target_hwnd,
        db,
        app,
    );
}

pub fn start_cache_rejection_monitor(
    injected_text: String,
    cache_key: String,
    target_hwnd: usize,
    db: DbHandle,
    app: AppHandle,
) {
    run_rejection_monitor(
        injected_text,
        RejectionTarget::CacheKey { key: cache_key },
        target_hwnd,
        db,
        app,
    );
}

#[cfg(test)]
mod tests {
    use super::{RejectionTarget, ResolvedContextIdentity};

    #[test]
    fn dictionary_rejection_target_keeps_mapping_ids_and_context_scope() {
        let target = RejectionTarget::DictionaryCorrections {
            correction_ids: vec![701, 702],
            context: ResolvedContextIdentity {
                t3_skills: None,
                id: 11,
                label: "Development".to_string(),
            },
        };

        assert_eq!(target.context_id(), Some(11));
        match target {
            RejectionTarget::DictionaryCorrections {
                correction_ids,
                context,
            } => {
                assert_eq!(correction_ids, vec![701, 702]);
                assert_eq!(context.id, 11);
            }
            RejectionTarget::CacheKey { .. } => unreachable!("wrong rejection target"),
        }
    }

    #[test]
    fn dictionary_rejection_monitor_keys_differ_between_contexts() {
        fn key(context_id: i64) -> String {
            let target = RejectionTarget::DictionaryCorrections {
                correction_ids: vec![701],
                context: ResolvedContextIdentity {
                    t3_skills: None,
                    id: context_id,
                    label: "context".to_string(),
                },
            };
            super::rejection_monitor_key("same injected text", &target)
        }

        assert_ne!(key(11), key(12));
    }
}
