use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use crate::core::context_probe::{
    describe_selection_state, resolve_context_from_tail, stable_metadata_hash, ContextProbeSource,
    InjectionContextProbe, SelectionState,
};
#[cfg(not(windows))]
use crate::core::context_probe::{ContextProbeSource, InjectionContextProbe};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::core::text_context::is_invisible_prefix_char as is_invisible_probe_char;
#[cfg(any(windows, test))]
use crate::core::text_context::SentenceContext;
use crate::data::{db, store};
use crate::DbHandle;

mod correction;
mod focused_text;
mod monitor;
mod rejection;
use correction::*;
// Glob feeds the `pub(super)` helpers (e.g. classify_caret_char,
// resolve_injection_context) to the test module via `use super::*`; on macOS
// the lib build uses none of them directly, so silence the unused-import lint.
#[allow(unused_imports)]
use focused_text::*;
#[allow(unused_imports)]
pub use focused_text::{
    read_focused_text, read_focused_text_probe, read_injection_context_probe, FocusedTextProbe,
};
pub use monitor::start_monitor;
use monitor::*;
pub use rejection::{start_cache_rejection_monitor, start_rejection_monitor};

const MONITOR_WINDOW_SECS: u64 = 60;
const POLL_INTERVAL_SECS: u64 = 1;
const BASELINE_CAPTURE_DELAY_MS: u64 = 250;
const BASELINE_RETRY_DELAY_MS: u64 = 500;
const EVENT_MONITOR_POLL_MS: u64 = 250;
const REJECTION_WINDOW_SECS: u64 = 8;
const REJECTION_POLL_MS: u64 = 500;
const CACHE_REJECTION_WINDOW_SECS: u64 = 10;
pub(crate) const PENDING_RETENTION_DAYS: i64 = 30;
const PROMOTION_THRESHOLD_DEFAULT: i64 = 2;
const PROMOTION_THRESHOLD_FAST: i64 = 1;
// candidate_confidence() tops out around 0.80; this is reachable only by
// distinctive corrections (brand/technical terms) with a small edit distance.
const FAST_PROMOTION_CONFIDENCE: f64 = 0.70;
const HIGH_CONFIDENCE_TIER: f64 = 0.70;
const MEDIUM_CONFIDENCE_TIER: f64 = 0.55;
const STABLE_TEXT_OBSERVATIONS_REQUIRED: usize = 2;
const STABLE_TEXT_MIN_DURATION: std::time::Duration = std::time::Duration::from_millis(750);
const MIN_CANDIDATE_NORM_LEN: usize = 2;
const MAX_SPAN_GROWTH_WORDS: usize = 5;
const MAX_REPLACEMENTS_PER_SPAN: usize = 2;
const MAX_CHANGED_OPS_PER_SPAN: usize = 4;
const MIN_CANDIDATE_CONFIDENCE: f64 = 0.45;

#[derive(Debug, Clone, PartialEq, Eq)]
struct WordToken {
    raw: String,
    norm: String,
}

#[derive(Debug, Clone, PartialEq)]
struct CandidateCorrection {
    mistake: String,
    correction: String,
    confidence: f64,
}

#[derive(Debug, Clone, Copy)]
struct CorrectionMetrics {
    a_len: usize,
    b_len: usize,
    max_len: usize,
    distance: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextAnchor {
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlignOp {
    Equal,
    Replace,
    Insert,
    Delete,
}

#[derive(Debug, Default)]
struct StableTextGate {
    pending_text: Option<String>,
    pending_observations: usize,
    pending_since: Option<std::time::Instant>,
    processed_text: Option<String>,
}

impl StableTextGate {
    fn observe(&mut self, text: String) -> Option<&str> {
        self.observe_at(text, std::time::Instant::now())
    }

    fn observe_at(&mut self, text: String, now: std::time::Instant) -> Option<&str> {
        if self.pending_text.as_deref() == Some(text.as_str()) {
            self.pending_observations += 1;
        } else {
            self.pending_text = Some(text);
            self.pending_observations = 1;
            self.pending_since = Some(now);
        }

        if self.pending_observations < STABLE_TEXT_OBSERVATIONS_REQUIRED
            || self.pending_since.is_none_or(|since| {
                now.checked_duration_since(since)
                    .is_none_or(|elapsed| elapsed < STABLE_TEXT_MIN_DURATION)
            })
        {
            return None;
        }

        let stable_text = self.pending_text.as_deref()?;
        if self.processed_text.as_deref() == Some(stable_text) {
            return None;
        }

        self.processed_text = Some(stable_text.to_string());
        self.processed_text.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::ResolvedContextIdentity;
    use crate::core::text_context::SentenceContext;
    use crate::data::db;

    fn test_context(id: i64) -> ResolvedContextIdentity {
        ResolvedContextIdentity {
            id,
            label: format!("Context {id}"),
        }
    }

    #[derive(Debug, serde::Deserialize)]
    struct AutoLearnCase {
        name: String,
        original: String,
        corrected: String,
        expect: Vec<[String; 2]>,
        /// Number of sessions needed to promote this pair, or `null` if the
        /// case isn't expected to promote at all. 1 exercises the
        /// fast-promotion path (confidence >= FAST_PROMOTION_CONFIDENCE);
        /// 2 exercises the default path.
        #[serde(default)]
        promotion_sessions: Option<u8>,
    }

    #[test]
    fn anchored_text_ignores_surrounding_content() {
        let injected = "Ask me about Koobernetes today";
        let baseline = "Before this. Ask me about Koobernetes today After this.";
        let current = "Before this. Ask me about Kubernetes today After this.";

        let diffs = detect_corrections_from_anchored_text(injected, baseline, current);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].mistake, "Koobernetes");
        assert_eq!(diffs[0].correction, "Kubernetes");
        assert!(diffs[0].confidence > 0.0);
    }

    #[test]
    fn reversed_surrounding_context_is_rejected_without_panicking() {
        let injected = "Ask about Koobernetes";
        let baseline = format!("left-boundary {injected} right-boundary");
        let current = " right-boundary Ask about Kubernetesleft-boundary ";
        assert!(detect_corrections_from_anchored_text(injected, &baseline, current).is_empty());
    }

    #[test]
    fn long_document_windows_with_one_origin_keep_correction_aligned() {
        let prefix = "earlier context ".repeat(400);
        let suffix = " later context".repeat(400);
        let injected = "Ask me about Koobernetes today";
        let baseline = format!("{prefix}{injected}{suffix}");
        let current = format!("{prefix}Ask me about Kubernetes today{suffix}");

        let diffs = detect_corrections_from_anchored_text(injected, &baseline, &current);
        assert_eq!(
            diffs
                .into_iter()
                .map(|candidate| (candidate.mistake, candidate.correction))
                .collect::<Vec<_>>(),
            vec![("Koobernetes".to_string(), "Kubernetes".to_string())]
        );
    }

    #[test]
    fn shifted_windows_keep_a_length_changing_correction_anchored() {
        let before = format!(
            "start {}",
            (0..180)
                .map(|index| format!("left{index} "))
                .collect::<String>()
        );
        let after = format!(
            "{} end",
            (0..180)
                .map(|index| format!("right{index} "))
                .collect::<String>()
        );
        let injected = "Ask me about Koobernetes today";
        let baseline = format!("{before}{injected}{after}");
        let current = format!("{}Ask me about Kubernetes today{}", &before[48..], after);

        let diffs = detect_corrections_from_anchored_text(injected, &baseline, &current);
        assert_eq!(
            diffs
                .into_iter()
                .map(|candidate| (candidate.mistake, candidate.correction))
                .collect::<Vec<_>>(),
            vec![("Koobernetes".to_string(), "Kubernetes".to_string())]
        );
    }

    #[test]
    fn long_injection_is_not_lost_when_the_window_moves() {
        let before = format!(
            "begin {}",
            (0..200)
                .map(|index| format!("left{index} "))
                .collect::<String>()
        );
        let injected = format!("{} Koobernetes", "dictated ".repeat(350));
        let corrected = format!("{} Kubernetes", "dictated ".repeat(350));
        let after = format!(
            "{} finish",
            (0..200)
                .map(|index| format!("right{index} "))
                .collect::<String>()
        );
        let baseline = format!("{before}{injected}{after}");
        let current = format!("{}{}{}", &before[48..], corrected, after);

        assert_eq!(
            detect_corrections_from_anchored_text(&injected, &baseline, &current)
                .into_iter()
                .map(|candidate| (candidate.mistake, candidate.correction))
                .collect::<Vec<_>>(),
            vec![("Koobernetes".to_string(), "Kubernetes".to_string())]
        );
    }

    #[test]
    fn edits_before_injected_span_are_ignored() {
        let injected = "Ask me about Koobernetes today";
        let baseline = "Before this. Ask me about Koobernetes today After this.";
        let current = "Before this please. Ask me about Kubernetes today After this.";

        assert!(detect_corrections_from_anchored_text(injected, baseline, current).is_empty());
    }

    #[test]
    fn simple_typo_correction_is_detected() {
        assert_eq!(
            diff_words(
                "please use Koobernetes today",
                "please use Kubernetes today"
            ),
            vec![("Koobernetes".to_string(), "Kubernetes".to_string())]
        );
    }

    #[test]
    fn casing_and_punctuation_only_changes_are_rejected() {
        assert!(diff_words("Ask.", "Ask").is_empty());
        assert!(diff_words("ask", "Ask").is_empty());
    }

    #[test]
    fn inserted_words_do_not_shift_later_alignment() {
        assert_eq!(
            diff_words(
                "please use Koobernetes today",
                "please use the Kubernetes today"
            ),
            vec![("Koobernetes".to_string(), "Kubernetes".to_string())]
        );
    }

    #[test]
    fn full_sentence_rewrites_are_ignored() {
        assert!(diff_words(
            "please use Koobernetes in the deployment tomorrow",
            "rewrite this whole sentence into something else"
        )
        .is_empty());
    }

    #[test]
    fn long_dictations_with_a_local_correction_stay_bounded() {
        let prefix = (0..20_000)
            .map(|index| format!("word{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let original = format!("{prefix} Koobernetes today");
        let corrected = format!("{prefix} Kubernetes today");

        assert_eq!(
            diff_words(&original, &corrected),
            vec![("Koobernetes".to_string(), "Kubernetes".to_string())]
        );
    }

    #[test]
    fn short_common_word_swaps_are_rejected() {
        assert!(diff_words("as me later", "ask me later").is_empty());
    }

    #[test]
    fn plain_suffix_completion_is_rejected() {
        assert!(diff_words("bran rot hostin", "bran rot hosting").is_empty());
        assert!(diff_words("send the file", "sends the file").is_empty());
        assert!(diff_words("say nugga", "say nuggaaaa").is_empty());
        assert!(diff_words("scratch the cat", "scratch the cats").is_empty());
        assert!(diff_words("we should do", "we should doing").is_empty());
    }

    #[test]
    fn short_technical_brand_term_correction_is_detected() {
        assert_eq!(
            diff_words("bran rock hosting", "bran qroq hosting"),
            vec![("rock".to_string(), "qroq".to_string())]
        );
        assert_eq!(
            diff_words("bran rot hosting", "bran qroq hosting"),
            vec![("rot".to_string(), "qroq".to_string())]
        );
    }

    #[test]
    fn one_letter_candidate_is_rejected() {
        assert!(diff_words("use x hosting", "use qroq hosting").is_empty());
        assert!(diff_words("use rock hosting", "use q hosting").is_empty());
    }

    #[test]
    fn repeated_candidate_counts_once_per_session() {
        let db = db::open(":memory:").expect("test db");
        let mut recorded = HashSet::new();

        assert!(!record_candidate(
            &db,
            &mut recorded,
            &test_context(1),
            "Koobernetes".to_string(),
            "Kubernetes".to_string(),
            0.6,
        ));
        assert!(!record_candidate(
            &db,
            &mut recorded,
            &test_context(1),
            "Koobernetes".to_string(),
            "Kubernetes".to_string(),
            0.6,
        ));

        let count = db::count_pending_corrections_recent_for_context(
            &db,
            db::EVERYWHERE_CONTEXT_ID,
            "Koobernetes",
            "Kubernetes",
            PENDING_RETENTION_DAYS,
        )
        .expect("pending count");
        assert_eq!(count, 1);
    }

    #[test]
    fn candidate_promotes_after_two_sessions() {
        let db = db::open(":memory:").expect("test db");

        for expected in [false, true] {
            let mut recorded = HashSet::new();
            assert_eq!(
                record_candidate(
                    &db,
                    &mut recorded,
                    &test_context(1),
                    "Koobernetes".to_string(),
                    "Kubernetes".to_string(),
                    0.6,
                ),
                expected
            );
        }

        let entries = db::query_dictionary(&db).expect("dictionary");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].term, "Kubernetes");
        assert_eq!(entries[0].mistake.as_deref(), Some("Koobernetes"));
        assert!(entries[0].auto_learned);
    }

    #[test]
    fn short_technical_term_promotes_only_after_two_sessions() {
        let db = db::open(":memory:").expect("test db");

        for expected in [false, true] {
            let mut recorded = HashSet::new();
            assert_eq!(
                record_candidate(
                    &db,
                    &mut recorded,
                    &test_context(1),
                    "rock".to_string(),
                    "qroq".to_string(),
                    0.6,
                ),
                expected
            );
        }

        let entries = db::query_dictionary(&db).expect("dictionary");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].term, "qroq");
        assert_eq!(entries[0].mistake.as_deref(), Some("rock"));
        assert!(entries[0].auto_learned);
    }

    #[test]
    fn high_confidence_candidate_promotes_after_one_session() {
        let db = db::open(":memory:").expect("test db");
        let mut recorded = HashSet::new();

        assert!(record_candidate(
            &db,
            &mut recorded,
            &test_context(1),
            "vsc0de".to_string(),
            "vscode".to_string(),
            0.75,
        ));

        let entries = db::query_dictionary(&db).expect("dictionary");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].term, "vscode");
        assert_eq!(entries[0].mistake.as_deref(), Some("vsc0de"));
        assert!(entries[0].auto_learned);
        assert_eq!(entries[0].confidence_tier, "high");
    }

    #[test]
    fn monitor_keys_are_scoped_to_the_resolved_context() {
        let development = test_context(2);
        let writing = test_context(3);

        assert_eq!(
            monitor_key("same injected phrase", &development),
            monitor_key("same injected phrase", &development)
        );
        assert_ne!(
            monitor_key("same injected phrase", &development),
            monitor_key("same injected phrase", &writing)
        );
    }

    #[test]
    fn candidate_session_keys_are_scoped_to_the_resolved_context() {
        let development = test_context(2);
        let writing = test_context(3);

        assert_eq!(
            candidate_session_key(&development, "Koobernetes", "Kubernetes"),
            candidate_session_key(&development, "Koobernetes", "Kubernetes")
        );
        assert_ne!(
            candidate_session_key(&development, "Koobernetes", "Kubernetes"),
            candidate_session_key(&writing, "Koobernetes", "Kubernetes")
        );
    }

    #[test]
    fn stable_text_gate_waits_for_consecutive_identical_reads() {
        let mut gate = StableTextGate::default();
        let mut now = std::time::Instant::now();
        let mut observe = |text: &str| {
            now += STABLE_TEXT_MIN_DURATION;
            gate.observe_at(text.to_string(), now).map(str::to_owned)
        };

        assert_eq!(observe("say nugga"), None);
        assert_eq!(observe("say nuggaaaa"), None);
        assert_eq!(observe("say nuggaaaaa"), None);
        assert_eq!(observe("say nuggaaaa"), None);
        assert_eq!(observe("say nuggaaaa"), Some("say nuggaaaa".to_string()));
        assert_eq!(observe("say nuggaaaa"), None);
    }

    #[test]
    fn fast_event_samples_do_not_learn_unfinished_typing() {
        let mut gate = StableTextGate::default();
        let now = std::time::Instant::now();
        assert_eq!(gate.observe_at("use OpenA".into(), now), None);
        assert_eq!(
            gate.observe_at(
                "use OpenA".into(),
                now + std::time::Duration::from_millis(250)
            ),
            None
        );
        assert_eq!(
            gate.observe_at(
                "use OpenAI".into(),
                now + std::time::Duration::from_millis(500)
            ),
            None
        );
        assert_eq!(
            gate.observe_at(
                "use OpenAI".into(),
                now + std::time::Duration::from_millis(1250)
            ),
            Some("use OpenAI")
        );
    }

    #[test]
    fn out_of_order_stability_timestamps_do_not_pass_or_panic() {
        let mut gate = StableTextGate::default();
        let since = std::time::Instant::now();
        assert_eq!(gate.observe_at("use OpenAI".into(), since), None);
        assert_eq!(
            gate.observe_at(
                "use OpenAI".into(),
                since
                    .checked_sub(std::time::Duration::from_millis(1))
                    .unwrap()
            ),
            None
        );
        assert_eq!(
            gate.observe_at("use OpenAI".into(), since + STABLE_TEXT_MIN_DURATION),
            Some("use OpenAI")
        );
    }

    #[test]
    fn blank_field_probe_capitalizes() {
        let context = resolve_injection_context(true, None);
        assert_eq!(context, SentenceContext::NewSentence);
        assert!(context.should_capitalize());
    }

    #[test]
    fn unknown_probe_capitalizes() {
        let context = resolve_injection_context(false, None);
        assert_eq!(context, SentenceContext::Unknown);
        assert!(context.should_capitalize());
    }

    #[test]
    fn sentence_ending_characters_capitalize() {
        assert_eq!(
            resolve_injection_context(false, Some('.')),
            SentenceContext::NewSentence
        );
        assert_eq!(
            resolve_injection_context(false, Some('!')),
            SentenceContext::NewSentence
        );
        assert_eq!(
            resolve_injection_context(false, Some('?')),
            SentenceContext::NewSentence
        );
    }

    #[test]
    fn mid_sentence_probe_lowercases() {
        let context = resolve_injection_context(false, Some('a'));
        assert_eq!(context, SentenceContext::MidSentence);
        assert!(!context.should_capitalize());
    }

    #[test]
    fn blank_field_overrides_stale_context() {
        assert_eq!(
            resolve_injection_context(true, Some('a')),
            SentenceContext::NewSentence
        );
    }

    #[test]
    fn invisible_probe_characters_do_not_force_lowercase() {
        assert_eq!(classify_caret_char('\u{200b}'), None);
        assert_eq!(classify_caret_char('\u{feff}'), None);
        assert_eq!(classify_caret_char('"'), None);
        assert_eq!(classify_caret_char('('), None);
        assert_eq!(
            resolve_injection_context(false, Some('\u{200b}')),
            SentenceContext::Unknown
        );
        assert_eq!(
            resolve_injection_context(false, Some('"')),
            SentenceContext::Unknown
        );
    }

    #[test]
    fn event_mode_hook_unavailable_uses_poll_interval_sleep() {
        assert_eq!(
            event_mode_poll_sleep_duration(false),
            std::time::Duration::from_secs(POLL_INTERVAL_SECS)
        );
    }

    #[test]
    fn event_mode_hook_ready_uses_fast_poll_sleep() {
        assert_eq!(
            event_mode_poll_sleep_duration(true),
            std::time::Duration::from_millis(EVENT_MONITOR_POLL_MS)
        );
    }

    #[test]
    fn event_mode_fallback_reads_without_a_hook() {
        assert!(event_mode_should_read(false, false, false));
    }

    #[test]
    fn event_mode_reads_again_after_one_edit_to_reach_stability() {
        assert!(event_mode_should_read(true, false, true));
        assert!(event_mode_should_read(true, true, false));
        assert!(!event_mode_should_read(true, false, false));
    }

    #[test]
    fn auto_learn_regression_matrix() {
        let raw = include_str!("../../testdata/auto_learn_cases.json");
        let cases: Vec<AutoLearnCase> = serde_json::from_str(raw).expect("valid cases");

        for case in cases {
            let actual = diff_words(&case.original, &case.corrected);
            let expected: Vec<(String, String)> = case
                .expect
                .iter()
                .map(|pair| (pair[0].clone(), pair[1].clone()))
                .collect();
            assert_eq!(actual, expected, "case {}", case.name);

            if let Some(sessions) = case.promotion_sessions {
                assert!(
                    !expected.is_empty(),
                    "case {} marked promotable without expected pair",
                    case.name
                );
                let db = db::open(":memory:").expect("test db");
                let candidate = detect_span_corrections(&case.original, &case.corrected)
                    .into_iter()
                    .next()
                    .expect("detected correction");
                let (mistake, correction, confidence) = (
                    candidate.mistake,
                    candidate.correction,
                    candidate.confidence,
                );

                for session in 1..=sessions {
                    let mut recorded = HashSet::new();
                    let promoted = record_candidate(
                        &db,
                        &mut recorded,
                        &test_context(1),
                        mistake.clone(),
                        correction.clone(),
                        confidence,
                    );
                    assert_eq!(
                        promoted,
                        session == sessions,
                        "case {} promotion threshold (session {session}/{sessions})",
                        case.name
                    );
                }
            }
        }
    }

    #[test]
    fn learned_brand_spelling_is_used_on_the_next_dictation() {
        for (before, after) in [
            ("call the api today", "call the API today"),
            ("use github today", "use GitHub today"),
            ("use open ai today", "use OpenAI today"),
        ] {
            let db = db::open(":memory:").unwrap();
            let candidate = detect_span_corrections(before, after).pop().unwrap();
            assert!(record_candidate(
                &db,
                &mut HashSet::new(),
                &test_context(1),
                candidate.mistake,
                candidate.correction,
                candidate.confidence
            ));
            let entries = db::query_dictionary_for_context(&db, 1).unwrap();
            let (output, applied) =
                crate::data::dictionary::apply_substitutions_from(before, &entries);
            assert_eq!(output, after);
            assert_eq!(applied.len(), 1);
            let (output, applied) =
                crate::data::dictionary::apply_substitutions_from(after, &entries);
            assert_eq!(output, after);
            assert!(
                applied.is_empty(),
                "an already correct spelling is not a correction"
            );
        }
    }

    #[test]
    fn occasional_corrections_accumulate_across_weeks() {
        let db = db::open(":memory:").unwrap();
        let candidate = detect_span_corrections("use Koobernetes today", "use Kubernetes today")
            .pop()
            .unwrap();
        assert!(!record_candidate(
            &db,
            &mut HashSet::new(),
            &test_context(1),
            candidate.mistake.clone(),
            candidate.correction.clone(),
            candidate.confidence
        ));
        db.lock()
            .unwrap()
            .execute(
                "UPDATE pending_corrections SET created_at = datetime('now', '-7 days')",
                [],
            )
            .unwrap();
        db::prune_pending_corrections(&db, PENDING_RETENTION_DAYS).unwrap();
        assert!(record_candidate(
            &db,
            &mut HashSet::new(),
            &test_context(1),
            candidate.mistake,
            candidate.correction,
            candidate.confidence
        ));
        assert_eq!(db::query_dictionary_for_context(&db, 1).unwrap().len(), 1);
    }

    #[test]
    fn oversized_words_are_not_compared_or_saved() {
        assert!(detect_span_corrections(&"x".repeat(20_000), &"X".repeat(20_000)).is_empty());
        assert!(detect_span_corrections(&"x".repeat(20_000), &"q".repeat(20_000)).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "Requires a live Hyprland desktop and Python GTK3"]
    fn auto_learn_native_disposable_fields() {
        use std::io::{BufRead, BufReader, Write};
        use std::process::{Child, Command, Stdio};
        crate::core::atspi::ensure_accessibility_enabled();
        std::thread::sleep(std::time::Duration::from_millis(250));
        fn read(text: &str, baseline: bool, pid: u32, stage: &str) -> MonitorText {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
            loop {
                if rejection::is_target_window_focused(pid as usize) {
                    if let Some(snapshot) = read_monitor_text(text, baseline) {
                        return snapshot;
                    }
                } else {
                    Command::new("hyprctl")
                        .args(["dispatch", "focuswindow", &format!("pid:{pid}")])
                        .stdout(Stdio::null())
                        .status()
                        .unwrap();
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "native accessibility read timed out during {stage}"
                );
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        struct Fixture(Child);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/test/linux-format-fixture.py");
        let mut fixture = Fixture(
            Command::new("python3")
                .env_remove("NO_AT_BRIDGE")
                .arg(script)
                .arg("--auto-learn")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let pid = fixture.0.id();
        let mut stdout = BufReader::new(fixture.0.stdout.take().unwrap());
        let mut ready = String::new();
        stdout.read_line(&mut ready).unwrap();
        assert!(ready.starts_with("[INFO] Fixture PID:"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !rejection::is_target_window_focused(pid as usize) {
            assert!(
                std::time::Instant::now() < deadline,
                "fixture must become focused"
            );
            Command::new("hyprctl")
                .args(["dispatch", "focuswindow", &format!("pid:{pid}")])
                .stdout(Stdio::null())
                .status()
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let mut edit = |field: usize, text: &str| {
            writeln!(
                fixture.0.stdin.as_mut().unwrap(),
                "{}",
                serde_json::json!({"field": field, "text": text})
            )
            .unwrap();
            let mut response = String::new();
            stdout.read_line(&mut response).unwrap();
            assert!(response.starts_with("[SUCCESS]"));
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        for (before, after, sessions) in [
            ("call the api today", "call the API today", 1),
            ("use github today", "use GitHub today", 1),
            ("use open ai today", "use OpenAI today", 1),
            ("use Koobernetes today", "use Kubernetes today", 2),
        ] {
            let db = db::open(":memory:").unwrap();
            for session in 1..=sessions {
                edit(0, before);
                let baseline = read(before, true, pid, "insertion baseline");
                assert!(find_unique_anchor(&baseline.text, before).is_some());
                edit(1, after);
                let other = read(before, false, pid, "alternate field");
                assert!(
                    !baseline.identity.matches(&other.identity),
                    "ignore another field in the same app"
                );
                edit(0, after);
                let corrected = read(before, false, pid, "corrected field");
                assert!(baseline.identity.matches(&corrected.identity));
                let mut gate = StableTextGate::default();
                let now = std::time::Instant::now();
                assert!(gate.observe_at(corrected.text.clone(), now).is_none());
                let stable = gate
                    .observe_at(corrected.text, now + STABLE_TEXT_MIN_DURATION)
                    .unwrap();
                let candidate =
                    detect_corrections_from_anchored_text(before, &baseline.text, stable)
                        .pop()
                        .unwrap();
                assert_eq!(
                    record_candidate(
                        &db,
                        &mut HashSet::new(),
                        &test_context(1),
                        candidate.mistake,
                        candidate.correction,
                        candidate.confidence
                    ),
                    session == sessions
                );
            }
            let entries = db::query_dictionary_for_context(&db, 1).unwrap();
            assert_eq!(entries.len(), 1);
            assert!(
                crate::data::dictionary::build_relevant_dictionary_prompt_from(&entries, before)
                    .contains(&entries[0].term)
            );
        }
        edit(2, "synthetic-password");
        assert!(
            read_monitor_text("", false).is_none(),
            "secure fields must not be read"
        );
    }
}
