use crate::analytics::{Analytics, FailureCategory, Stage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// Owns one run's correlation across awaits and generation changes. Never
/// look up the current recording's ID after another take may have started.
pub(super) struct PipelineTelemetry {
    analytics: Option<Analytics>,
    run_id: Option<String>,
    stage: Mutex<(Stage, Instant)>,
    finished: AtomicBool,
}

impl PipelineTelemetry {
    pub(super) fn new(analytics: Option<Analytics>, run_id: Option<String>) -> Self {
        Self {
            analytics,
            run_id,
            stage: Mutex::new((Stage::Capture, Instant::now())),
            finished: AtomicBool::new(false),
        }
    }

    fn with_run(&self, f: impl FnOnce(&Analytics, &str)) {
        if let (Some(analytics), Some(run_id)) = (&self.analytics, &self.run_id) {
            if !analytics.run_has_final_outcome(run_id) {
                f(analytics, run_id);
            }
        }
    }

    pub(super) fn start_stage(&self, stage: Stage) {
        if let Ok(mut current) = self.stage.lock() {
            *current = (stage, Instant::now());
        }
        self.with_run(|analytics, id| analytics.pipeline_stage_started(id, stage));
    }

    pub(super) fn complete_stage(&self, provider_model: Option<&str>) {
        if let Ok(current) = self.stage.lock() {
            self.with_run(|analytics, id| {
                analytics.pipeline_stage_completed_with_model(
                    id,
                    current.0,
                    current.1.elapsed().as_millis(),
                    provider_model,
                );
            });
        }
    }

    pub(super) fn recording_finished(&self, duration_ms: u64) {
        self.with_run(|analytics, id| analytics.recording_finished(id, duration_ms));
    }

    pub(super) fn context(&self, matched: bool, manual: bool) {
        self.with_run(|analytics, id| {
            analytics.context_outcome(id, matched, if manual { "manual" } else { "automatic" });
        });
    }

    pub(super) fn feature(&self, feature: &'static str) {
        self.with_run(|analytics, id| analytics.feature_used(id, feature));
    }

    pub(super) fn input_health(&self, outcome: &'static str) {
        self.with_run(|analytics, _| analytics.input_health(outcome));
    }

    pub(super) fn insertion_attempted(&self) {
        self.start_stage(Stage::Insertion);
        self.with_run(|analytics, id| analytics.insertion_attempted(id));
    }

    pub(super) fn delivered(&self, method: &'static str, words: i64) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        self.complete_stage(None);
        self.with_run(|analytics, id| {
            if method == "clipboard_fallback" {
                analytics.fallback_used(id, "clipboard");
            }
            analytics.dictation_inserted(id, method, words);
            analytics.delivery_outcome(id, "success_clean");
        });
    }

    pub(super) fn failed(&self, category: FailureCategory, rejected: bool) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        let stage = self.stage.lock().map(|v| v.0).unwrap_or(Stage::Unknown);
        self.with_run(|analytics, id| {
            analytics.pipeline_failed(id, stage, category);
            analytics.delivery_outcome(
                id,
                if rejected {
                    "rejected_expected"
                } else {
                    "failure_terminal"
                },
            );
        });
    }

    pub(super) fn cancelled(&self) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        self.with_run(|analytics, id| {
            analytics.dictation_cancelled(id, false);
            analytics.delivery_outcome(id, "cancelled_user");
        });
    }
}

impl Drop for PipelineTelemetry {
    fn drop(&mut self) {
        // Early returns and unwinding must not leave an accepted run without
        // a terminal outcome. Specific paths finish with a bounded category.
        self.failed(FailureCategory::Internal, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> (Analytics, PipelineTelemetry) {
        let path = std::env::temp_dir().join(format!("verenu-telemetry-{}", uuid::Uuid::new_v4()));
        let analytics = Analytics::new(false, path);
        let id = analytics.new_run_id();
        analytics.dictation_started(&id, false, false);
        let telemetry = PipelineTelemetry::new(Some(analytics.clone()), Some(id));
        (analytics, telemetry)
    }

    #[test]
    fn delivered_run_keeps_correlation_and_has_one_final_outcome() {
        let (analytics, telemetry) = run();
        telemetry.recording_finished(4000);
        telemetry.start_stage(Stage::Transcription);
        telemetry.complete_stage(Some("groq/whisper-large-v3/transcription"));
        telemetry.insertion_attempted();
        telemetry.delivered("event_only", 42);
        telemetry.delivered("direct_insertion", 99);
        drop(telemetry);
        let events = analytics.captured_events();
        let run_id = &events[0].1["run_id"];
        for (_, properties) in &events {
            assert_eq!(&properties["run_id"], run_id);
        }
        let outcomes: Vec<_> = events
            .iter()
            .filter(|e| e.0 == "dictation_outcome")
            .collect();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].1["status"], "success");
        assert_eq!(outcomes[0].1["word_count"], 42);
        assert_eq!(outcomes[0].1["recording_duration_ms"], 4000);
        assert_eq!(outcomes[0].1["transcription_provider"], "groq");
        assert_eq!(
            events
                .iter()
                .filter(|e| e.0 == "dictation_inserted")
                .count(),
            1
        );
    }

    #[test]
    fn early_return_cancel_and_rejection_each_close_the_run() {
        for expected in ["failure", "cancelled", "rejected"] {
            let (analytics, telemetry) = run();
            match expected {
                "cancelled" => telemetry.cancelled(),
                "rejected" => telemetry.failed(FailureCategory::VadRejected, true),
                _ => {}
            }
            drop(telemetry);
            let events = analytics.captured_events();
            let outcomes: Vec<_> = events
                .iter()
                .filter(|e| e.0 == "dictation_outcome")
                .collect();
            assert_eq!(outcomes.len(), 1);
            assert_eq!(outcomes[0].1["status"], expected);
            assert!(!events.iter().any(|e| e.0 == "dictation_inserted"));
            if expected == "failure" {
                let failure = events.iter().find(|e| e.0 == "pipeline_failed").unwrap();
                assert_eq!(failure.1["stage"], "capture");
            }
        }
    }

    #[test]
    fn external_cancellation_prevents_late_delivery_and_drop_failure() {
        let (analytics, telemetry) = run();
        let id = telemetry.run_id.as_deref().unwrap();
        analytics.dictation_cancelled(id, true);
        analytics.delivery_outcome(id, "cancelled_user");
        telemetry.delivered("direct_insertion", 7);
        drop(telemetry);
        let events = analytics.captured_events();
        assert_eq!(
            events.iter().filter(|e| e.0 == "dictation_outcome").count(),
            1
        );
        assert!(!events
            .iter()
            .any(|e| matches!(e.0, "dictation_inserted" | "pipeline_failed")));
    }

    #[test]
    fn clipboard_fallback_is_recovered_success_and_failed_copy_is_not_delivery() {
        let (analytics, telemetry) = run();
        telemetry.insertion_attempted();
        telemetry.delivered("clipboard_fallback", 7);
        drop(telemetry);
        let events = analytics.captured_events();
        let outcome = events.iter().find(|e| e.0 == "dictation_outcome").unwrap();
        assert_eq!(outcome.1["outcome"], "success_after_clipboard_fallback");
        assert_eq!(outcome.1["recovered"], true);

        let (analytics, telemetry) = run();
        telemetry.failed(FailureCategory::InsertionFailed, false);
        telemetry.delivered("clipboard_fallback", 7);
        drop(telemetry);
        assert!(!analytics
            .captured_events()
            .iter()
            .any(|e| e.0 == "dictation_inserted"));
    }
}
