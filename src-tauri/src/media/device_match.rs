//! Match a stored microphone name to WASAPI/CPAL device names.
//!
//! The same hardware often appears as `Microphone (Yeti Nano)`, `Yeti Nano`,
//! or `Microphone (Yeti Nano) (2)` after a replug. Exact string equality
//! misses those, so mute watching would bind the default input instead.

pub const SCORE_NONE: u8 = 0;
pub const SCORE_CONTAINS: u8 = 1;
pub const SCORE_NORMALIZED: u8 = 2;
pub const SCORE_CASEFOLD: u8 = 3;
pub const SCORE_EXACT: u8 = 4;

/// Longer prefixes first so "headset microphone" wins over "headset".
const ROLE_PREFIXES: &[&str] = &[
    "headset microphone",
    "headset mic",
    "microphone array",
    "microphone",
    "headphones",
    "headset",
    "line in",
    "line",
    "mic",
];

/// Generic labels that would false-match under substring comparison.
const GENERIC_NORMALIZED: &[&str] = &[
    "usb audio device",
    "usb audio",
    "generic usb audio",
    "microphone",
    "headset",
    "headphones",
    "realtek audio",
    "high definition audio device",
    "microsoft sound mapper",
    "primary sound capture driver",
];

#[allow(dead_code)]
pub fn names_match(candidate: &str, desired: &str) -> bool {
    device_name_match_score(candidate, desired) > SCORE_NONE
}

pub fn best_score_against(desired: &str, candidates: &[&str]) -> u8 {
    candidates
        .iter()
        .map(|candidate| device_name_match_score(candidate, desired))
        .max()
        .unwrap_or(SCORE_NONE)
}

pub fn device_name_match_score(candidate: &str, desired: &str) -> u8 {
    if candidate == desired {
        return SCORE_EXACT;
    }
    if candidate.eq_ignore_ascii_case(desired) {
        return SCORE_CASEFOLD;
    }
    let nc = normalize_device_name(candidate);
    let nd = normalize_device_name(desired);
    if nc.is_empty() || nd.is_empty() {
        return SCORE_NONE;
    }
    if nc == nd {
        return SCORE_NORMALIZED;
    }
    if is_generic(&nc) || is_generic(&nd) {
        return SCORE_NONE;
    }
    let (shorter, longer) = if nc.len() <= nd.len() {
        (&nc, &nd)
    } else {
        (&nd, &nc)
    };
    if shorter.len() >= 6 && longer.contains(shorter.as_str()) {
        return SCORE_CONTAINS;
    }
    SCORE_NONE
}

pub fn normalize_device_name(name: &str) -> String {
    let collapsed = collapse_ws(&name.to_lowercase());
    let stripped = strip_instance_suffix(&collapsed);
    unwrap_role_wrapper(stripped).to_string()
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_instance_suffix(s: &str) -> &str {
    if !s.ends_with(')') {
        return s;
    }
    if let Some(open) = s.rfind(" (") {
        let inner = &s[open + 2..s.len() - 1];
        if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()) {
            return &s[..open];
        }
    }
    s
}

fn unwrap_role_wrapper(s: &str) -> &str {
    for prefix in ROLE_PREFIXES {
        if let Some(rest) = s.strip_prefix(prefix) {
            let rest = rest.trim();
            if rest.starts_with('(') && rest.ends_with(')') && rest.len() >= 3 {
                return rest[1..rest.len() - 1].trim();
            }
        }
    }
    s
}

fn is_generic(normalized: &str) -> bool {
    GENERIC_NORMALIZED.contains(&normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_name_is_highest_score() {
        assert_eq!(
            device_name_match_score("Microphone (Yeti Nano)", "Microphone (Yeti Nano)"),
            SCORE_EXACT
        );
    }

    #[test]
    fn unwraps_microphone_role_prefix() {
        assert_eq!(
            device_name_match_score("Microphone (Yeti Nano)", "Yeti Nano"),
            SCORE_NORMALIZED
        );
        assert_eq!(
            normalize_device_name("Microphone (Yeti Nano)"),
            "yeti nano"
        );
    }

    #[test]
    fn unwraps_headset_microphone_and_instance_suffix() {
        assert_eq!(
            device_name_match_score(
                "Headset Microphone (Jabra Evolve 75) (2)",
                "Microphone (Jabra Evolve 75)"
            ),
            SCORE_NORMALIZED
        );
    }

    #[test]
    fn bluetooth_product_substring_matches() {
        assert_eq!(
            device_name_match_score(
                "Headset (WH-1000XM5 Hands-Free AG Audio)",
                "WH-1000XM5"
            ),
            SCORE_CONTAINS
        );
    }

    #[test]
    fn generic_usb_audio_does_not_substring_match() {
        assert_eq!(
            device_name_match_score(
                "Microphone (USB Audio Device)",
                "Microphone (Other USB Audio Device)"
            ),
            SCORE_NONE
        );
    }

    #[test]
    fn distinct_mics_do_not_match() {
        assert_eq!(
            device_name_match_score("Microphone (Yeti Nano)", "Microphone (Elgato Wave:3)"),
            SCORE_NONE
        );
    }

    #[test]
    fn case_insensitive_exact() {
        assert_eq!(
            device_name_match_score("Yeti Nano", "yeti nano"),
            SCORE_CASEFOLD
        );
    }

    #[test]
    fn names_match_helper() {
        assert!(names_match("Microphone (Yeti Nano)", "Yeti Nano"));
        assert!(!names_match("Microphone (Yeti Nano)", "Shure MV7"));
    }
}
