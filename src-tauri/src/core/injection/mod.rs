use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static INJECTION_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
pub(super) fn injection_lock() -> &'static tokio::sync::Mutex<()> {
    INJECTION_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

use crate::core::context_probe::{ContextProbeSource, InjectionContextProbe, SelectionState};
use crate::core::text_context;
#[cfg(any(desktop, test))]
mod chunks;
#[cfg(any(desktop, test))]
pub(crate) use chunks::ChunkedPasteError;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod linux;

const HISTORY_TAIL: usize = 512;

#[cfg(target_os = "windows")]
const CLIPBOARD_OPEN_RETRY_MS: u64 = 50;
#[cfg(target_os = "windows")]
const CLIPBOARD_RESTORE_RETRY_MS: u64 = 60;
#[cfg(target_os = "windows")]
const CLIPBOARD_RESTORE_ATTEMPTS: usize = 3;

#[cfg(target_os = "windows")]
const REFOCUS_SETTLE_MS: u64 = 150;

#[cfg(target_os = "windows")]
const CLIPBOARD_WRITE_SETTLE_MS: u64 = 50;

#[cfg(target_os = "windows")]
const CLIPBOARD_WRITE_RETRY_MS: u64 = 50;

#[cfg(target_os = "windows")]
const MODIFIER_GAP_MS: u64 = 30;

#[cfg(target_os = "windows")]
const WIN_KEY_GRACE_POLL_MS: u64 = 30;
#[cfg(target_os = "windows")]
const WIN_KEY_GRACE_POLL_ATTEMPTS: u32 = 5;

#[cfg(target_os = "windows")]
const WIN_KEY_RELEASE_SETTLE_MS: u64 = 80;

#[cfg(target_os = "windows")]
const PASTE_SETTLE_MS: u64 = 250;

#[cfg(test)]
const SENTENCE_ENDERS: &[char] = &['.', '!', '?', '\n', '\r'];
#[derive(Clone, Debug)]
enum CursorContextState {
    Unknown {
        _instant: Instant,
    },
    Known {
        hwnd: usize,
        tail: String,
        instant: Instant,
    },
}

#[cfg_attr(not(windows), allow(dead_code))]
const PASTE_VERIFY_SUFFIX_CHARS: usize = 20;

#[cfg_attr(not(windows), allow(dead_code))]
const PASTE_VERIFY_CONTAINS_MIN_CHARS: usize = 12;

#[cfg_attr(not(windows), allow(dead_code))]
fn injected_suffix(injected: &str) -> String {
    let mut chars: Vec<char> = injected
        .trim_end()
        .chars()
        .rev()
        .take(PASTE_VERIFY_SUFFIX_CHARS)
        .collect();
    chars.reverse();
    chars.into_iter().collect()
}

#[cfg_attr(not(windows), allow(dead_code))]
fn paste_tail_matches(injected: &str, probe_tail: &str) -> bool {
    if injected.trim_end().is_empty() {
        return true;
    }
    probe_tail.trim_end().ends_with(&injected_suffix(injected))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn full_text_confirms_paste(injected: &str, full_text: &str) -> bool {
    let injected_trim = injected.trim_end();
    if injected_trim.is_empty() {
        return true;
    }
    let suffix = injected_suffix(injected);
    if full_text.trim_end().ends_with(&suffix) {
        return true;
    }
    suffix.chars().count() >= PASTE_VERIFY_CONTAINS_MIN_CHARS && full_text.contains(&suffix)
}

#[cfg(test)]
mod tests {
    #[test]
    fn caret_text_formatting_matches_the_desktop_rules() {
        let fmt = |left: &str, right: &str, text: &str| {
            format_for_caret_text(text, left, right, true, true, true, true, "casual", "en", false)
        };
        assert_eq!(fmt("", "", "hello there"), "Hello there");
        assert_eq!(fmt("Hello.", "", "next sentence"), " Next sentence");
        assert_eq!(fmt("Hello", "", "World"), " world");
        assert_eq!(fmt("Hello ", "", "World"), "world");
        assert_eq!(
            format_for_caret_text("World", "Hello", "", true, true, false, false, "casual", "en", false),
            "World"
        );
        assert_eq!(
            format_for_caret_text("world", "", "", false, false, true, true, "casual", "en", false),
            "world"
        );
    }

    #[test]
    fn native_cursor_formatting_preserves_terminal_t3_skill_token() {
        let catalog = crate::system::t3_skills::Catalog {
            environment_id: "synthetic".into(),
            id: "catalog".into(),
            label: "Codex".into(),
            provider_instance_id: "codex".into(),
            workspace_id: "synthetic".into(),
            revision: "1".into(),
            skills: vec![crate::system::t3_skills::Skill {
                name: "babysit-pr".into(),
                display_name: None,
                description: None,
            }],
        };
        for (spoken, expected) in [
            ("$BABYSIT-PR?", "$babysit-pr ? "),
            ("$BABYSIT-PR", "$babysit-pr "),
        ] {
            let normalized = catalog.normalize_mentions(spoken);
            assert_eq!(normalized, expected);
            assert_eq!(
                format_for_caret_text(
                    &normalized,
                    "Previous sentence. ",
                    "",
                    true,
                    true,
                    true,
                    true,
                    "casual",
                    "en",
                    catalog.starts_with_skill_mention(&normalized),
                ),
                expected
            );
        }
    }

    use super::*;

    #[test]
    fn classify_context_detects_sentence_boundary() {
        assert!(matches!(
            classify_context("Hello. "),
            ContextKind::SentenceBoundary
        ));
        assert!(matches!(
            classify_context("Hello?\n"),
            ContextKind::SentenceBoundary
        ));
        assert!(matches!(
            classify_context(""),
            ContextKind::SentenceBoundary
        ));
    }

    #[test]
    fn classify_context_detects_continuation() {
        assert!(matches!(
            classify_context("hello, "),
            ContextKind::Continuation
        ));
        assert!(matches!(
            classify_context("path/to/"),
            ContextKind::Continuation
        ));
        assert!(matches!(
            classify_context("label:"),
            ContextKind::Continuation
        ));
    }

    #[test]
    fn contextual_caps_disabled_preserves_caps_lock_uppercased_text() {
        let probe = InjectionContextProbe {
            context: crate::core::text_context::SentenceContext::MidSentence,
            source: ContextProbeSource::CaretLocal,
            context_tail: "hello ".to_string(),
            context_head: String::new(),
            left_reliable: true,
            right_reliable: true,
            selection_state: SelectionState::CollapsedCaret,
            control_identity_hash: "test".to_string(),
            control_type: "test".to_string(),
            target_id: 1,
        };
        let (adjusted, _, case_decision) = apply_probe_adjustments(
            "THE REPORT IS READY",
            false,
            false,
            "casual",
            "en",
            false,
            &probe,
        );
        assert_eq!(adjusted, "THE REPORT IS READY");
        assert!(matches!(
            case_decision,
            CaseDecision::ContextualCapsDisabled
        ));
    }

    #[test]
    fn confirmed_unfinished_text_lowercases_an_ordinary_leading_capital() {
        let probe = InjectionContextProbe {
            context: crate::core::text_context::SentenceContext::MidSentence,
            source: ContextProbeSource::CaretLocal,
            context_tail: "unfinished".to_string(),
            context_head: String::new(),
            left_reliable: true,
            right_reliable: true,
            selection_state: SelectionState::CollapsedCaret,
            control_identity_hash: "test".to_string(),
            control_type: "test".to_string(),
            target_id: 1,
        };
        let (adjusted, context, case_decision) =
            apply_probe_adjustments("Hello again", true, true, "casual", "en", false, &probe);
        assert_eq!(adjusted, " hello again");
        assert!(matches!(context, ContextKind::Continuation));
        assert!(matches!(
            case_decision,
            CaseDecision::ContinuationLowercased
        ));
    }

    #[test]
    fn confirmed_comma_continuation_lowercases_even_before_another_titlecase_word() {
        let probe = InjectionContextProbe {
            context: crate::core::text_context::SentenceContext::MidSentence,
            source: ContextProbeSource::CaretLocal,
            context_tail: "Yabba Dabba Dooba,".to_string(),
            context_head: String::new(),
            left_reliable: true,
            right_reliable: true,
            selection_state: SelectionState::CollapsedCaret,
            control_identity_hash: "test".to_string(),
            control_type: "test".to_string(),
            target_id: 1,
        };
        let (adjusted, context, case_decision) =
            apply_probe_adjustments("Dabba Doo.", true, true, "casual", "en", false, &probe);
        assert_eq!(adjusted, " dabba Doo.");
        assert!(matches!(context, ContextKind::Continuation));
        assert!(matches!(
            case_decision,
            CaseDecision::ContinuationLowercased
        ));
    }

    #[test]
    fn unconfirmed_left_edge_never_capitalizes_or_adds_leading_space() {
        let probe = InjectionContextProbe {
            context: crate::core::text_context::SentenceContext::NewSentence,
            source: ContextProbeSource::CaretLocal,
            context_tail: String::new(),
            context_head: "existing".to_string(),
            left_reliable: false,
            right_reliable: true,
            selection_state: SelectionState::CollapsedCaret,
            control_identity_hash: "test".to_string(),
            control_type: "test".to_string(),
            target_id: 1,
        };
        let (adjusted, context, case_decision) =
            apply_probe_adjustments("hello", true, true, "casual", "en", false, &probe);
        assert_eq!(adjusted, "hello ");
        assert!(matches!(context, ContextKind::Unknown));
        assert!(matches!(
            case_decision,
            CaseDecision::ConservativeDegradePreserved
        ));
    }

    #[test]
    fn history_tracks_manual_typing_and_backspace_by_window() {
        reset_injection_history();
        append_or_reset_injection_history(11, 'h');
        append_or_reset_injection_history(11, 'i');

        let state = last_injection().lock().expect("lock");
        match &*state {
            CursorContextState::Known { hwnd, tail, .. } => {
                assert_eq!(*hwnd, 11);
                assert_eq!(tail, "hi");
            }
            CursorContextState::Unknown { .. } => panic!("expected known state"),
        }
        drop(state);

        backspace_injection_history(22);
        let state = last_injection().lock().expect("lock");
        assert!(matches!(&*state, CursorContextState::Unknown { .. }));
    }

    #[test]
    fn paste_tail_matches_matches_our_suffix() {
        assert!(paste_tail_matches("hello world", "some prefix hello world"));
        assert!(paste_tail_matches("hello world", "hello world"));
        assert!(paste_tail_matches("hello world ", "prefix hello world\n"));
        let long = "the quick brown fox jumps over the lazy dog near the river";
        assert!(paste_tail_matches(
            long,
            "stale pre-paste text the quick brown fox jumps over the lazy dog near the river"
        ));
        assert!(paste_tail_matches("", "anything at all"));
        assert!(!paste_tail_matches("hello world", "some other text"));
    }

    #[test]
    fn full_text_confirms_paste_checks_whole_document() {
        assert!(full_text_confirms_paste(
            "dictated text here",
            "existing prompt dictated text here"
        ));
        assert!(full_text_confirms_paste(
            "dictated text here",
            "start dictated text here rest of document"
        ));
        assert!(full_text_confirms_paste("short", "abc short"));
        assert!(!full_text_confirms_paste("short", "abc shortx"));
        assert!(full_text_confirms_paste("", "anything at all"));
        assert!(!full_text_confirms_paste(
            "didn't paste",
            "totally unrelated text"
        ));
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextKind {
    Unknown,
    SentenceBoundary,
    Continuation,
}
impl ContextKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::SentenceBoundary => "sentence_boundary",
            Self::Continuation => "continuation",
        }
    }
}

fn context_kind_from_sentence_context(
    context: crate::core::text_context::SentenceContext,
) -> ContextKind {
    match context {
        crate::core::text_context::SentenceContext::NewSentence => ContextKind::SentenceBoundary,
        crate::core::text_context::SentenceContext::MidSentence => ContextKind::Continuation,
        crate::core::text_context::SentenceContext::Unknown => ContextKind::Unknown,
    }
}
#[derive(Clone, Copy, Debug)]
enum CaseDecision {
    ContextualCapsDisabled,
    ConservativeDegradePreserved,
    UnknownContextPreserved,
    SentenceBoundaryCapitalized,
    SentenceBoundaryPreservedVeryCasual,
    ContinuationLowercased,
    ContinuationPreserved,
}
impl CaseDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::ContextualCapsDisabled => "contextual_caps_disabled",
            Self::ConservativeDegradePreserved => "conservative_degrade_preserved",
            Self::UnknownContextPreserved => "unknown_context_preserved",
            Self::SentenceBoundaryCapitalized => "sentence_boundary_capitalized",
            Self::SentenceBoundaryPreservedVeryCasual => "sentence_boundary_preserved_very_casual",
            Self::ContinuationLowercased => "continuation_lowercased",
            Self::ContinuationPreserved => "continuation_preserved",
        }
    }
}
pub struct InjectionOutcome {
    pub text: String,
    pub context_state: &'static str,
    pub case_decision: &'static str,
    pub probe_source: &'static str,
    pub selection_state: &'static str,
}
static LAST_INJECTION: OnceLock<Mutex<CursorContextState>> = OnceLock::new();
fn unknown_context() -> CursorContextState {
    CursorContextState::Unknown {
        _instant: Instant::now(),
    }
}
fn last_injection() -> &'static Mutex<CursorContextState> {
    LAST_INJECTION.get_or_init(|| Mutex::new(unknown_context()))
}
fn trim_tail_to_limit(text: &mut String) {
    if text.len() > HISTORY_TAIL {
        let excess = text.len() - HISTORY_TAIL;
        let mut trim_at = excess;
        while !text.is_char_boundary(trim_at) {
            trim_at += 1;
        }
        *text = text[trim_at..].to_owned();
    }
}
#[cfg(test)]
fn trimmed_context_for_decision(text: &str) -> &str {
    text.trim_end_matches(|c: char| c.is_whitespace() && !SENTENCE_ENDERS.contains(&c))
}
#[cfg(test)]
fn classify_context(tail: &str) -> ContextKind {
    let trimmed = trimmed_context_for_decision(tail);
    if trimmed
        .chars()
        .next_back()
        .map(|c| SENTENCE_ENDERS.contains(&c))
        .unwrap_or(true)
    {
        ContextKind::SentenceBoundary
    } else {
        ContextKind::Continuation
    }
}
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn unavailable_injection_probe() -> InjectionContextProbe {
    InjectionContextProbe::unavailable(ContextProbeSource::Unavailable, "unavailable")
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
pub fn format_for_caret_text(
    text: &str,
    left: &str,
    right: &str,
    left_reliable: bool,
    right_reliable: bool,
    contextual_caps: bool,
    auto_spacing: bool,
    profile: &str,
    language: &str,
    protected_initial_case: bool,
) -> String {
    let empty = left.is_empty() && right.is_empty() && left_reliable && right_reliable;
    let probe = InjectionContextProbe {
        context: text_context::SentenceContext::Unknown,
        source: if empty {
            ContextProbeSource::EmptyField
        } else {
            ContextProbeSource::CaretLocal
        },
        context_tail: left.to_owned(),
        context_head: right.to_owned(),
        left_reliable,
        right_reliable,
        selection_state: SelectionState::CollapsedCaret,
        control_identity_hash: String::new(),
        control_type: "android_edit_text".to_owned(),
        target_id: 0,
    };
    apply_probe_adjustments(
        text,
        contextual_caps,
        auto_spacing,
        profile,
        language,
        protected_initial_case,
        &probe,
    )
    .0
}

fn apply_probe_adjustments(
    text: &str,
    contextual_caps: bool,
    auto_spacing: bool,
    profile: &str,
    language: &str,
    protected_initial_case: bool,
    probe: &InjectionContextProbe,
) -> (String, ContextKind, CaseDecision) {
    let formatting_enabled = contextual_caps || auto_spacing;
    let source_reliable = probe.source == ContextProbeSource::EmptyField
        || (probe.source == ContextProbeSource::CaretLocal
            && !matches!(probe.selection_state, SelectionState::Unknown));
    let left_reliable = formatting_enabled && source_reliable && probe.left_reliable;
    let right_reliable = formatting_enabled && source_reliable && probe.right_reliable;
    let context_kind = if left_reliable {
        context_kind_from_sentence_context(probe.context)
    } else {
        ContextKind::Unknown
    };
    let decision = text_context::decide_insertion(
        text,
        text_context::CaretTextContext {
            left: &probe.context_tail,
            right: &probe.context_head,
            left_reliable,
            right_reliable,
            language,
            casing_enabled: contextual_caps,
            preserve_sentence_case: profile == "very_casual",
            protected_initial_case,
        },
    );
    let adjusted = decision.text;
    let case_decision = if !contextual_caps {
        CaseDecision::ContextualCapsDisabled
    } else if !left_reliable {
        CaseDecision::ConservativeDegradePreserved
    } else {
        match decision.case_action {
            text_context::CaseAction::CapitalizeFirstWord => {
                CaseDecision::SentenceBoundaryCapitalized
            }
            text_context::CaseAction::LowercaseFirstWord => CaseDecision::ContinuationLowercased,
            text_context::CaseAction::Preserve
                if context_kind == ContextKind::SentenceBoundary && profile == "very_casual" =>
            {
                CaseDecision::SentenceBoundaryPreservedVeryCasual
            }
            text_context::CaseAction::Preserve if context_kind == ContextKind::Unknown => {
                CaseDecision::UnknownContextPreserved
            }
            text_context::CaseAction::Preserve => CaseDecision::ContinuationPreserved,
        }
    };

    log::debug!(
        "injection: smart-format control_type={} probe_source={} context={} tail_len={} head_len={} left_reliable={} right_reliable={} tail_signal={} head_signal={} case_decision={} leading_space={} trailing_space={} reason={}",
        probe.control_type,
        probe.source.as_str(),
        probe.context.as_str(),
        probe.context_tail.chars().count(),
        probe.context_head.chars().count(),
        left_reliable,
        right_reliable,
        context_tail_signal(&probe.context_tail),
        context_head_signal(&probe.context_head),
        case_decision.as_str(),
        decision.leading_space,
        decision.trailing_space,
        decision.reason,
    );

    (adjusted, context_kind, case_decision)
}

fn context_head_signal(head: &str) -> &'static str {
    match head.chars().find(|c| !c.is_whitespace()) {
        None => {
            if head.contains('\n') || head.contains('\r') {
                "newline_only"
            } else {
                "empty_or_ws"
            }
        }
        Some(ch) if ch.is_alphanumeric() => "alnum",
        Some(')' | ']' | '}' | '.' | ',' | ';' | ':' | '!' | '?') => "punct",
        Some(_) => "other",
    }
}

fn context_tail_signal(tail: &str) -> &'static str {
    match tail.chars().rev().find(|c| !c.is_whitespace()) {
        None => {
            if tail.contains('\n') || tail.contains('\r') {
                "newline_only"
            } else {
                "empty_or_ws"
            }
        }
        Some('.') | Some('!') | Some('?') => "sentence_end",
        Some(ch) if ch.is_alphanumeric() => "alnum",
        Some(',') | Some(';') | Some(':') | Some('-') | Some('–') | Some('—') | Some('/')
        | Some('\\') => "soft_punct",
        Some(_) => "other",
    }
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub fn reset_injection_history() {
    if let Ok(mut guard) = last_injection().lock() {
        *guard = unknown_context();
    }
}
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub fn backspace_injection_history(hwnd: usize) {
    if let Ok(mut guard) = last_injection().lock() {
        match &mut *guard {
            CursorContextState::Known {
                hwnd: stored_hwnd,
                tail,
                instant,
            } if *stored_hwnd == hwnd => {
                tail.pop();
                *instant = Instant::now();
                if tail.is_empty() {
                    *guard = unknown_context();
                }
            }
            _ => {
                *guard = unknown_context();
            }
        }
    }
}
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
pub fn append_or_reset_injection_history(hwnd: usize, ch: char) {
    if let Ok(mut guard) = last_injection().lock() {
        match &mut *guard {
            CursorContextState::Known {
                hwnd: stored_hwnd,
                tail,
                instant,
            } if *stored_hwnd == hwnd => {
                tail.push(ch);
                trim_tail_to_limit(tail);
                *instant = Instant::now();
            }
            _ => {
                let mut tail = ch.to_string();
                trim_tail_to_limit(&mut tail);
                *guard = CursorContextState::Known {
                    hwnd,
                    tail,
                    instant: Instant::now(),
                };
            }
        }
    }
}
pub async fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let _guard = injection_lock().lock().await;
    #[cfg(windows)]
    {
        return windows::copy_to_clipboard(text).await;
    }
    #[cfg(target_os = "macos")]
    {
        return macos::copy_to_clipboard(text).await;
    }
    #[cfg(target_os = "linux")]
    {
        return linux::copy_to_clipboard(text).await;
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
        anyhow::bail!("copy_to_clipboard: unsupported platform")
    }
}

pub async fn read_current_clipboard_text() -> Option<String> {
    #[cfg(windows)]
    {
        windows::read_clipboard_text().await
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[allow(unused_variables)]
#[allow(clippy::too_many_arguments)]
pub async fn inject_text(
    text: &str,
    target: &crate::core::window_geometry::WindowTarget,
    contextual_caps: bool,
    auto_spacing: bool,
    profile: &str,
    language: &str,
    protected_initial_case: bool,
    paste_in_chunks: bool,
) -> anyhow::Result<InjectionOutcome> {
    #[cfg(any(test, debug_assertions))]
    if crate::testing::injection_capture_enabled() {
        crate::testing::record_injection(crate::testing::InjectionRecord {
            text: text.to_string(),
            target_hwnd: target.id,
            contextual_caps,
            auto_spacing,
            profile: profile.to_string(),
            paste_in_chunks: cfg!(desktop) && paste_in_chunks,
        });
        return Ok(InjectionOutcome {
            text: text.to_string(),
            context_state: "test_mode",
            case_decision: "test_mode_passthrough",
            probe_source: "test_harness",
            selection_state: "unknown",
        });
    }

    #[cfg(target_os = "windows")]
    {
        return windows::inject_text(
            text,
            target.id,
            contextual_caps,
            auto_spacing,
            profile,
            language,
            protected_initial_case,
            paste_in_chunks,
        )
        .await;
    }

    #[cfg(target_os = "macos")]
    {
        return macos::inject_text(
            text,
            target.id,
            contextual_caps,
            auto_spacing,
            profile,
            language,
            protected_initial_case,
            paste_in_chunks,
        )
        .await;
    }

    #[cfg(target_os = "linux")]
    {
        return linux::inject_text(
            text, target, contextual_caps, auto_spacing, profile, language, protected_initial_case, paste_in_chunks,
        ).await;
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        #[cfg(target_os = "android")]
        {
            let seq = crate::android::bridge::publish_android_insertion_with(
                text,
                Some(crate::android::bridge::InsertionFormat {
                    contextual_caps,
                    auto_spacing,
                    profile: profile.to_string(),
                    language: language.to_string(),
                    protected_initial_case,
                }),
            );
            log::info!(
                "inject_text(android): handoff seq={seq} target={} chars={}", target.id,
                text.chars().count()
            );
            return Ok(InjectionOutcome {
                text: text.to_string(),
                context_state: "android",
                case_decision: "android_accessibility_handoff",
                probe_source: "android_accessibility",
                selection_state: "unknown",
            });
        }
        #[cfg(not(target_os = "android"))]
        {
            log::warn!("inject_text: unsupported target={}", target.id);

            Ok(InjectionOutcome {
                text: text.to_string(),
                context_state: "unknown",
                case_decision: "contextual_caps_disabled",
                probe_source: "unavailable",
                selection_state: "unknown",
            })
        }
    }
}
