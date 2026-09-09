use super::*;
use crate::core::context::ResolvedContextIdentity;

static ACTIVE_MONITORS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Eq, Hash, PartialEq)]
pub(super) struct CandidateSessionKey {
    context_id: i64,
    mistake: String,
    correction: String,
}

pub(super) fn candidate_session_key(
    context: &ResolvedContextIdentity,
    mistake: &str,
    correction: &str,
) -> CandidateSessionKey {
    CandidateSessionKey {
        context_id: context.id,
        mistake: mistake.to_owned(),
        correction: correction.to_owned(),
    }
}

fn log_context_event(
    db: &DbHandle,
    context: &ResolvedContextIdentity,
    event_type: &str,
    reason_code: &str,
    mistake_hash: &str,
    correction_hash: &str,
    confidence: f64,
) {
    // Keep the stable numeric identity in persistence while retaining the
    // existing display label for local diagnostics. The label is never used
    // for scope decisions and no raw text is passed to the event logger.
    let _ = db::log_auto_learn_event_for_context(
        db,
        context.id,
        db::AutoLearnEventFields {
            event_type,
            reason_code,
            app_context: &context.label,
            mistake_hash,
            correction_hash,
            confidence,
        },
    );
}

pub(super) fn active_monitors() -> &'static Mutex<HashSet<String>> {
    ACTIVE_MONITORS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) struct MonitorKeyGuard {
    key: String,
}

impl MonitorKeyGuard {
    pub(super) fn new(key: String) -> Self {
        Self { key }
    }
}

impl Drop for MonitorKeyGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = active_monitors().lock() {
            active.remove(&self.key);
        }
    }
}

#[cfg(test)]
pub(super) fn diff_words(original: &str, current: &str) -> Vec<(String, String)> {
    detect_span_corrections(original, current)
        .into_iter()
        .map(|c| (c.mistake, c.correction))
        .collect()
}

pub(super) fn record_candidate(
    db: &DbHandle,
    recorded_this_session: &mut HashSet<CandidateSessionKey>,
    context: &ResolvedContextIdentity,
    mistake: String,
    correction: String,
    confidence: f64,
) -> bool {
    let key = candidate_session_key(context, &mistake, &correction);
    if recorded_this_session.contains(&key) {
        log_context_event(
            db,
            context,
            "candidate",
            "duplicate_in_session",
            "",
            "",
            confidence,
        );
        return false;
    }
    recorded_this_session.insert(key);
    let (mistake_hash, correction_hash) = pair_hash(&mistake, &correction);

    if confidence < MIN_CANDIDATE_CONFIDENCE {
        log_context_event(
            db,
            context,
            "candidate",
            "low_confidence",
            &mistake_hash,
            &correction_hash,
            confidence,
        );
        return false;
    }

    let confidence_avg =
        match db::upsert_auto_learn_candidate_for_context(
            db,
            context.id,
            &mistake,
            &correction,
            confidence,
        ) {
            Ok(confidence_avg) => confidence_avg,
            Err(e) => {
                log::warn!("auto-learn candidate upsert failed: {e}");
                log_context_event(
                    db,
                    context,
                    "candidate",
                    "candidate_upsert_failed",
                    &mistake_hash,
                    &correction_hash,
                    confidence,
                );
                return false;
            }
        };

    let tier = if confidence_avg >= HIGH_CONFIDENCE_TIER {
        "high"
    } else if confidence_avg >= MEDIUM_CONFIDENCE_TIER {
        "medium"
    } else {
        "low"
    };
    let threshold = if confidence_avg >= FAST_PROMOTION_CONFIDENCE {
        PROMOTION_THRESHOLD_FAST
    } else {
        PROMOTION_THRESHOLD_DEFAULT
    };

    // The pending insert, threshold count, `promoted_at`, and dictionary upsert
    // happen in ONE transaction inside the DB layer. Concurrent monitors
    // observing the same pair in this Context can no longer both pass the
    // threshold and both "promote" it (double events / inflated
    // correction_count), and a rejection that purges the Context-scoped
    // candidate mid-flight can no longer be undone by an in-flight promotion
    // recreating the rejected row.
    match db::auto_learn_promote_for_context(
        db,
        context.id,
        &mistake,
        &correction,
        tier,
        PENDING_RETENTION_DAYS,
        threshold,
    ) {
        Ok(db::AutoLearnPromoteResult::Promoted) => {
            log_context_event(
                db,
                context,
                "promotion",
                "promoted",
                &mistake_hash,
                &correction_hash,
                confidence,
            );
            true
        }
        Ok(db::AutoLearnPromoteResult::BelowThreshold { .. }) => {
            log_context_event(
                db,
                context,
                "candidate",
                "below_threshold",
                &mistake_hash,
                &correction_hash,
                confidence,
            );
            false
        }
        Ok(db::AutoLearnPromoteResult::Blocked) => {
            log::debug!("auto-learn: promotion skipped because a manual dictionary entry exists");
            log_context_event(
                db,
                context,
                "promotion",
                "promotion_skipped",
                &mistake_hash,
                &correction_hash,
                confidence,
            );
            false
        }
        Ok(db::AutoLearnPromoteResult::AlreadyPromoted) => {
            log::debug!(
                "auto-learn: promotion skipped \u{2014} a concurrent monitor or rejection already claimed this pair"
            );
            log_context_event(
                db,
                context,
                "promotion",
                "promotion_skipped",
                &mistake_hash,
                &correction_hash,
                confidence,
            );
            false
        }
        Err(e) => {
            log::warn!("auto-learn dictionary promotion failed: {e}");
            log_context_event(
                db,
                context,
                "promotion",
                "promotion_failed",
                &mistake_hash,
                &correction_hash,
                confidence,
            );
            false
        }
    }
}
pub(super) fn auto_learn_event_mode_enabled(app: &AppHandle) -> bool {
    store::settings_handle(app)
        .ok()
        .and_then(|settings| settings.get(store::AUTO_LEARN_EVENT_MODE))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub(super) fn event_mode_poll_sleep_duration(hook_ready: bool) -> std::time::Duration {
    if hook_ready {
        std::time::Duration::from_millis(EVENT_MONITOR_POLL_MS)
    } else {
        std::time::Duration::from_secs(POLL_INTERVAL_SECS)
    }
}

/// Event mode only gates reads while it is waiting for the first notification
/// about a change. Once an event has started an observation, later reads must
/// continue so the stable-text gate can see inactivity as a second sample.
pub(super) fn event_mode_should_read(
    hook_ready: bool,
    observation_pending: bool,
    event_seen: bool,
) -> bool {
    !hook_ready || observation_pending || event_seen
}

const MAX_ACTIVE_MONITORS: usize = 32;
const MONITOR_QUEUE_CAPACITY: usize = 64;

struct MonitorRequest {
    key: String,
    injected_text: String,
    context: ResolvedContextIdentity,
    db: DbHandle,
    app: AppHandle,
    event_mode: bool,
}

struct MonitorTask {
    _monitor_guard: MonitorKeyGuard,
    injected_text: String,
    context: ResolvedContextIdentity,
    db: DbHandle,
    app: AppHandle,
    event_mode: bool,
    baseline_text: Option<String>,
    baseline_attempts: u8,
    next_action: std::time::Instant,
    deadline: std::time::Instant,
    stable_text_gate: StableTextGate,
    // Keep the Context in the key even though one task currently belongs to
    // one Context. This prevents a future task/session reuse from silently
    // turning a pair-only deduplication set back into a global key.
    recorded_this_session: HashSet<CandidateSessionKey>,
    #[cfg(windows)]
    _event_mode_hook_guard: Option<EventModeHookGuard>,
    #[cfg(windows)]
    last_event_seq: u64,
    event_observation_pending: bool,
}

impl MonitorTask {
    fn new(request: MonitorRequest) -> Self {
        let now = std::time::Instant::now();
        if let Err(e) = db::prune_pending_corrections(&request.db, PENDING_RETENTION_DAYS) {
            log::warn!("auto-learn prune failed: {e}");
        }
        if let Err(e) = db::prune_auto_learn_retention(&request.db) {
            log::warn!("auto-learn bookkeeping retention failed: {e}");
        }
        log_context_event(
            &request.db,
            &request.context,
            "monitor",
            "started",
            "",
            "",
            0.0,
        );
        log_context_event(
            &request.db,
            &request.context,
            "monitor",
            if request.event_mode {
                "event_mode"
            } else {
                "poll_mode"
            },
            "",
            "",
            0.0,
        );
        Self {
            _monitor_guard: MonitorKeyGuard::new(request.key),
            injected_text: request.injected_text,
            context: request.context,
            db: request.db,
            app: request.app,
            event_mode: request.event_mode,
            baseline_text: None,
            baseline_attempts: 0,
            next_action: now + std::time::Duration::from_millis(BASELINE_CAPTURE_DELAY_MS),
            deadline: now + std::time::Duration::from_secs(MONITOR_WINDOW_SECS),
            stable_text_gate: StableTextGate::default(),
            recorded_this_session: HashSet::new(),
            #[cfg(windows)]
            _event_mode_hook_guard: request.event_mode.then(EventModeHookGuard::new),
            #[cfg(windows)]
            last_event_seq: VALUE_CHANGE_SEQ.load(Ordering::Relaxed),
            event_observation_pending: false,
        }
    }

    fn step(&mut self) -> bool {
        let now = std::time::Instant::now();
        if now >= self.deadline {
            log_context_event(&self.db, &self.context, "monitor", "timeout", "", "", 0.0);
            return true;
        }
        if now < self.next_action {
            return false;
        }

        if self.baseline_text.is_none() {
            self.baseline_attempts += 1;
            if let Some(baseline_text) = capture_baseline_text(&self.injected_text) {
                self.baseline_text = Some(baseline_text);
                log_context_event(&self.db, &self.context, "anchor", "anchor_ok", "", "", 0.0);
                self.next_action = now + std::time::Duration::from_secs(POLL_INTERVAL_SECS);
            } else if self.baseline_attempts < 2 {
                self.next_action = now + std::time::Duration::from_millis(BASELINE_RETRY_DELAY_MS);
            } else {
                log::debug!("auto-learn: could not anchor injected text in focused control");
                log_context_event(
                    &self.db,
                    &self.context,
                    "anchor",
                    "anchor_miss",
                    "",
                    "",
                    0.0,
                );
                return true;
            }
            return false;
        }

        if self.event_mode {
            let mut should_read = true;
            #[cfg(windows)]
            {
                if ensure_value_change_hook() {
                    let sequence = VALUE_CHANGE_SEQ.load(Ordering::Relaxed);
                    let event_seen = sequence != self.last_event_seq;
                    if event_seen {
                        self.last_event_seq = sequence;
                        self.event_observation_pending = true;
                    }
                    should_read =
                        event_mode_should_read(true, self.event_observation_pending, event_seen);
                    if !should_read {
                        self.next_action = now + event_mode_poll_sleep_duration(true);
                        return false;
                    }
                } else {
                    // A hook is an optimization, not a requirement. Keep the
                    // monitor alive and use the regular polling path if UIA
                    // could not install it.
                    self.event_mode = false;
                    log_context_event(
                        &self.db,
                        &self.context,
                        "monitor",
                        "event_mode_fallback",
                        "",
                        "",
                        0.0,
                    );
                }
            }
            #[cfg(not(windows))]
            {
                // There is no native value-change hook on this platform, so
                // event mode transparently falls back to polling.
                self.event_mode = false;
            }
            if self.event_mode && !should_read {
                self.next_action = now + event_mode_poll_sleep_duration(true);
                return false;
            }
        }

        let Some(current_text) = read_focused_text_near_caret(&self.injected_text) else {
            self.next_action = now
                + if self.event_mode {
                    event_mode_poll_sleep_duration(true)
                } else {
                    std::time::Duration::from_secs(POLL_INTERVAL_SECS)
                };
            return false;
        };
        let Some(stable_text) = self.stable_text_gate.observe(current_text) else {
            self.next_action = now
                + if self.event_mode {
                    event_mode_poll_sleep_duration(true)
                } else {
                    std::time::Duration::from_secs(POLL_INTERVAL_SECS)
                };
            return false;
        };
        log_context_event(
            &self.db,
            &self.context,
            "stable_text",
            "stable_pass",
            "",
            "",
            0.0,
        );

        let baseline_text = self.baseline_text.as_deref().unwrap_or_default();
        let diffs =
            detect_corrections_from_anchored_text(&self.injected_text, baseline_text, stable_text);
        for candidate in diffs {
            if record_candidate(
                &self.db,
                &mut self.recorded_this_session,
                &self.context,
                candidate.mistake,
                candidate.correction,
                candidate.confidence,
            ) {
                log::info!("auto-learn: promoted candidate pair");
                // Keep the originating Context on the event so the canonical
                // Contexts view can refresh precisely without treating a
                // targeted learning event as a global dictionary mutation.
                self.app
                    .emit(
                        "verenu:dictionary-updated",
                        serde_json::json!({ "context_id": self.context.id }),
                    )
                    .ok();
            }
        }
        self.event_observation_pending = false;
        self.next_action = now
            + if self.event_mode {
                event_mode_poll_sleep_duration(true)
            } else {
                std::time::Duration::from_secs(POLL_INTERVAL_SECS)
            };
        false
    }
}

fn coordinator_sender() -> &'static std::sync::mpsc::SyncSender<MonitorRequest> {
    static SENDER: OnceLock<std::sync::mpsc::SyncSender<MonitorRequest>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(MONITOR_QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name("auto_learn_coordinator".to_string())
            .spawn(move || run_coordinator(receiver))
            .expect("auto-learn coordinator thread");
        sender
    })
}

fn run_coordinator(receiver: std::sync::mpsc::Receiver<MonitorRequest>) {
    let mut tasks: Vec<MonitorTask> = Vec::new();
    loop {
        while let Ok(request) = receiver.try_recv() {
            tasks.push(MonitorTask::new(request));
        }

        if tasks.is_empty() {
            let Ok(request) = receiver.recv() else { return };
            tasks.push(MonitorTask::new(request));
            continue;
        }

        let mut index = 0;
        while index < tasks.len() {
            if tasks[index].step() {
                tasks.swap_remove(index);
            } else {
                index += 1;
            }
        }
        match receiver.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(request) => tasks.push(MonitorTask::new(request)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

pub fn start_monitor(
    injected_text: String,
    context: ResolvedContextIdentity,
    db: DbHandle,
    app: AppHandle,
) {
    if injected_text.split_whitespace().count() < 2 {
        log_context_event(&db, &context, "monitor", "too_short", "", "", 0.0);
        return;
    }
    let key = monitor_key(&injected_text, &context);
    let (inserted, rejection_event) = match active_monitors().lock() {
        Ok(active) if active.contains(&key) => (false, "duplicate_skip"),
        Ok(active) if active.len() >= MAX_ACTIVE_MONITORS => (false, "capacity_skip"),
        Ok(mut active) => (active.insert(key.clone()), "capacity_skip"),
        Err(_) => (false, "capacity_skip"),
    };
    if !inserted {
        log_context_event(&db, &context, "monitor", rejection_event, "", "", 0.0);
        return;
    }

    let event_mode = auto_learn_event_mode_enabled(&app);
    log_context_event(
        &db,
        &context,
        "monitor",
        if event_mode {
            "event_mode"
        } else {
            "poll_mode"
        },
        "",
        "",
        0.0,
    );
    let request = MonitorRequest {
        key,
        injected_text,
        context,
        db,
        app,
        event_mode,
    };
    if let Err(error) = coordinator_sender().try_send(request) {
        let request = match error {
            std::sync::mpsc::TrySendError::Full(request)
            | std::sync::mpsc::TrySendError::Disconnected(request) => request,
        };
        drop(MonitorKeyGuard::new(request.key));
    }
}
