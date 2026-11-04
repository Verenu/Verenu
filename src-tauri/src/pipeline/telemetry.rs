use crate::analytics::{Analytics, FailureCategory, Stage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

pub(super) struct PipelineTelemetry {
    analytics: Option<Analytics>,
    run_id: Option<String>,
    stage: Mutex<(Stage, Instant)>,
    finished: AtomicBool,
    started: Instant,
    log_stages: Mutex<Vec<serde_json::Value>>,
    log_failures: Mutex<Vec<serde_json::Value>>,
    usage: crate::api::usage::Collector,
}

impl PipelineTelemetry {
    pub(super) fn new(analytics: Option<Analytics>, run_id: Option<String>) -> Self {
        Self {
            analytics,
            run_id,
            stage: Mutex::new((Stage::Capture, Instant::now())),
            finished: AtomicBool::new(false),
            started: Instant::now(),
            log_stages: Mutex::new(Vec::new()),
            log_failures: Mutex::new(Vec::new()),
            usage: Default::default(),
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
        self.complete_stage_with_duration(provider_model, None);
    }

    pub(super) fn complete_stage_with_duration(&self, provider_model: Option<&str>, duration: Option<std::time::Duration>) {
        if let Ok(current) = self.stage.lock() {
            let elapsed = duration.unwrap_or_else(|| current.1.elapsed());
            if let Ok(mut stages) = self.log_stages.lock() {
                let name = if matches!(current.0, Stage::Insertion) { "injection" } else { current.0.as_str() };
                stages.push(serde_json::json!({"stage": name, "duration_ms": elapsed.as_millis().min(u64::MAX as u128) as u64, "status": "ok"}));
            }
            self.with_run(|analytics, id| {
                analytics.pipeline_stage_completed_with_model(
                    id,
                    current.0,
                    elapsed.as_millis(),
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

    pub(super) fn fallback(&self, kind: &'static str) {
        if let Ok(mut failures) = self.log_failures.lock() {
            failures.push(serde_json::json!({"stage": kind, "message": "A fallback was used."}));
        }
        self.with_run(|analytics, id| analytics.fallback_used(id, kind));
    }

    pub(super) fn cleanup_unavailable(&self) {
        if let Ok(mut failures) = self.log_failures.lock() {
            failures.push(serde_json::json!({"stage": "cleanup", "message": "Cleanup was unavailable. Your transcript was preserved."}));
        }
    }

    pub(super) fn retry(&self, attempt: u8, reason: &'static str) {
        self.with_run(|analytics, id| analytics.retry_attempted(id, attempt, reason));
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
        if matches!(stage, Stage::Insertion) {
            if let Ok(mut failures) = self.log_failures.lock() {
                failures.push(serde_json::json!({"stage": "injection", "message": "Verenu could not insert the text."}));
            }
        }
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

    pub(super) fn history_log(&self, entry_id: i64, audio_ms: u64, calls: &[crate::data::db::ApiCall]) -> serde_json::Value {
        let mut stages = self.log_stages.lock().map(|v| v.clone()).unwrap_or_default();
        let failures = self.log_failures.lock().map(|v| v.clone()).unwrap_or_default();
        if failures.iter().any(|failure| failure["message"] == "Cleanup was unavailable. Your transcript was preserved.") {
            if let Some(stage) = stages.iter_mut().find(|stage| stage["stage"] == "cleanup") {
                stage["status"] = serde_json::json!("failed");
            }
        }
        for call in calls {
            if let Some(stage) = stages.iter_mut().find(|s| s["stage"] == call.task && s.get("model").is_none()) {
                stage["provider"] = serde_json::json!(call.provider);
                stage["model"] = serde_json::json!(call.model);
            } else {
                stages.push(serde_json::json!({"stage": call.task, "provider": call.provider, "model": call.model}));
            }
        }
        if let Ok(usage) = self.usage.lock() {
            let mut totals: Vec<crate::api::usage::Usage> = Vec::new();
            for item in usage.iter() {
                if let Some(total) = totals.iter_mut().find(|entry| entry.stage == item.stage && entry.provider == item.provider && entry.model == item.model) {
                    total.input = total.input.zip(item.input).and_then(|(a, b)| a.checked_add(b));
                    total.output = total.output.zip(item.output).and_then(|(a, b)| a.checked_add(b));
                } else {
                    totals.push(item.clone());
                }
            }
            for item in totals {
                let stage = stages.iter_mut().find(|s| s["stage"] == item.stage && s["provider"] == item.provider && s["model"] == item.model && s.get("input_tokens").is_none() && s.get("output_tokens").is_none());
                if let Some(stage) = stage {
                    stage["input_tokens"] = serde_json::json!(item.input);
                    stage["output_tokens"] = serde_json::json!(item.output);
                } else {
                    stages.push(serde_json::json!({"stage": item.stage, "provider": item.provider, "model": item.model, "input_tokens": item.input, "output_tokens": item.output}));
                }
            }
        }
        serde_json::json!({"entry_id": entry_id, "audio_ms": audio_ms, "total_ms": self.started.elapsed().as_millis().min(u64::MAX as u128) as u64, "stages": stages, "failures": failures})
    }

    pub(super) fn usage_collector(&self) -> crate::api::usage::Collector { self.usage.clone() }
}

impl Drop for PipelineTelemetry {
    fn drop(&mut self) {
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
    fn history_log_uses_overlapped_vad_duration_and_freezes_at_delivery() {
        let (_, telemetry) = run();
        telemetry.start_stage(Stage::Vad);
        telemetry.complete_stage_with_duration(None, Some(std::time::Duration::from_millis(250)));
        telemetry.insertion_attempted();
        telemetry.delivered("direct_insertion", 1);
        let log_at_delivery = telemetry.history_log(1, 1000, &[]);
        std::thread::sleep(std::time::Duration::from_millis(40));
        assert_eq!(log_at_delivery["stages"][0]["duration_ms"], 250);
        assert!(telemetry.history_log(1, 1000, &[])["total_ms"].as_u64().unwrap() >= log_at_delivery["total_ms"].as_u64().unwrap() + 35);
        assert_eq!(telemetry.history_log(1, 1000, &[])["stages"][1]["duration_ms"], log_at_delivery["stages"][1]["duration_ms"]);
    }

    #[test]
    fn history_log_aggregates_retry_usage_and_records_cleanup_failure() {
        let (_, telemetry) = run();
        {
            let mut usage = telemetry.usage.lock().unwrap();
            for (input, output) in [(10, 2), (20, 4)] {
                usage.push(crate::api::usage::Usage { stage: "cleanup".into(), provider: "test".into(), model: "model".into(), input: Some(input), output: Some(output) });
            }
        }
        telemetry.start_stage(Stage::Cleanup);
        telemetry.complete_stage(None);
        telemetry.cleanup_unavailable();
        let log = telemetry.history_log(1, 1000, &[]);
        assert_eq!(log["stages"][0]["status"], "failed");
        assert_eq!(log["stages"][1]["input_tokens"], 30);
        assert_eq!(log["stages"][1]["output_tokens"], 6);
        assert_eq!(log["failures"].as_array().unwrap().len(), 1);
        assert_eq!(log["failures"][0]["message"], "Cleanup was unavailable. Your transcript was preserved.");
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
        telemetry.fallback("cleanup");
        telemetry.retry(2, "timeout");
        telemetry.delivered("direct_insertion", 7);
        drop(telemetry);
        let events = analytics.captured_events();
        assert_eq!(
            events.iter().filter(|e| e.0 == "dictation_outcome").count(),
            1
        );
        assert!(!events.iter().any(|e| matches!(
            e.0,
            "dictation_inserted" | "pipeline_failed" | "fallback_used" | "retry_attempted"
        )));
    }

    #[test]
    fn provider_recovery_keeps_original_run_when_a_new_take_starts() {
        let (analytics, telemetry) = run();
        let original = telemetry.run_id.as_deref().unwrap();
        let next = analytics.new_run_id();
        analytics.dictation_started(&next, false, false);
        telemetry.fallback("transcription");
        telemetry.fallback("cleanup");
        telemetry.retry(2, "timeout");
        telemetry.delivered("direct_insertion", 7);
        let events = analytics.captured_events();
        let recovery: Vec<_> = events
            .iter()
            .filter(|e| matches!(e.0, "fallback_used" | "retry_attempted"))
            .collect();
        assert_eq!(recovery.len(), 3);
        assert!(recovery.iter().all(|e| e.1["run_id"] == original));
        assert!(!analytics.run_has_final_outcome(&next));
        let outcome = events.iter().find(|e| e.0 == "dictation_outcome").unwrap();
        assert_eq!(outcome.1["run_id"], original);
        assert_eq!(outcome.1["transcription_fallback_used"], true);
        assert_eq!(outcome.1["cleanup_fallback_used"], true);
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
