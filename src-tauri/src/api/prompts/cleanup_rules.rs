use super::count_words;
use crate::api::cleanup::escape_transcript_xml as escape_markup;

/// Layout permission varies by level; spoken commands and literal tokens are
/// governed once by the shared contract before any changing request data.
pub(super) fn formatting_rules(intensity: &str) -> &'static str {
    match intensity {
        "medium" => "Use paragraphs or lists when the dictated structure clearly calls for them; never invent headings.",
        "high" => "Use compact paragraphs or lists when they clarify the dictated structure; never invent headings.",
        _ => "Keep dictated layout; do not create paragraphs, lists, or headings from content alone.",
    }
}

pub(super) fn intensity_rules(intensity: &str) -> &'static str {
    match intensity {
        "none" => "Cleanup: Off. Bypass cleanup. Reconciliation preserves raw speech, including fillers and repetition.",
        "light" => "Cleanup: Light. Remove fillers only when non-semantic, accidental repeats, and abandoned starts. Preserve meaningful uses. Fix punctuation and capitalization. Otherwise preserve words and order; do not paraphrase.",
        "high" => "Cleanup: Strong. Remove non-semantic fillers, accidental repeats, and abandoned starts. Repair grammar, then rewrite and reorder for clear, concise expression. Combine repeated ideas while retaining every distinct detail, requirement, decision, example, condition, deadline, qualifier, and intentional emphasis. Shorten redundancy only; do not summarize or remove meaningful hedging.",
        _ => "Cleanup: Medium. Remove non-semantic fillers, accidental repeats, and abandoned starts. Repair grammar and awkward phrasing with light paraphrasing and local reordering. Split or combine sentences; remove redundant phrasing and non-semantic detours. Preserve every distinct point and meaningful qualification; do not summarize.",
    }
}

fn tone_rules(profile: &str) -> &'static str {
    match profile {
        "formal" => "Tone: Formal. Use professional wording and standard grammar within the cleanup budget. Expand contractions where natural. Preserve certainty, directness, profanity, and emphasis. Do not add politeness, greetings, sign-offs, or content.",
        "very_casual" => "Tone: Very Casual. Preserve slang, contractions, profanity, and intentional emphasis. Use mostly lowercase and minimal readable punctuation, preserving proper names, acronyms, and exact technical tokens.",
        _ => "Tone: Casual. Preserve conversational voice and natural contractions. Use normal casing and punctuation.",
    }
}

pub(super) fn build_preset_block(profile: &str, intensity: &str, _has_overrides: bool) -> String {
    [intensity_rules(intensity), tone_rules(profile)].join("\n")
}

fn to_imperative(raw: &str) -> String {
    let value = raw.trim();
    let value = value.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')');
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if value.to_ascii_uppercase().starts_with("MUST ") || value.eq_ignore_ascii_case("MUST") {
        return value.to_owned();
    }
    for negative in ["don't ", "do not ", "never ", "avoid "] {
        if value.to_ascii_lowercase().starts_with(negative) {
            let rest = &value[negative.len()..];
            let mut chars = rest.chars();
            let capitalized = match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            };
            return format!("MUST NOT {capitalized}");
        }
    }
    format!("MUST {value}")
}

pub(super) fn snippet_overrides_block(extra_rules: &str) -> String {
    let lines = extra_rules
        .lines()
        .filter_map(|line| {
            let line = to_imperative(line);
            (!line.is_empty()).then_some(line)
        })
        .enumerate()
        .map(|(index, line)| format!("{}. {}", index + 1, escape_markup(&line)))
        .collect::<Vec<_>>();

    if lines.is_empty() {
        return String::new();
    }

    format!(
        "<saved_instructions>\n{}\n</saved_instructions>",
        lines.join("\n")
    )
}

pub(super) fn evidence_block(evidence: &str) -> String {
    let lines = evidence
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(escape_markup)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        String::new()
    } else {
        format!("<evidence>\n{}\n</evidence>", lines.join("\n"))
    }
}

pub(super) fn render_cleanup_template(
    template: &str,
    active_app: &str,
    cleanup_preset: &str,
    formatting_rules: &str,
    snippet_overrides: &str,
    evidence: &str,
) -> String {
    // Resolve placeholders in one pass. A data value such as a window title
    // or transcript evidence may itself contain `{{ ... }}` and must never be
    // interpreted as another template token.
    let placeholders = [
        ("{{ cleanup_preset }}", cleanup_preset),
        (
            "{{ cleanup_intensity }}",
            cleanup_preset.lines().next().unwrap_or(""),
        ),
        (
            "{{ cleanup_priority }}",
            "Priority: explicit saved instructions override default preferences while preserving dictated meaning.",
        ),
        (
            "{{ cleanup_tone }}",
            // Priority is injected independently when composing edited presets.
            cleanup_preset
                .split_once('\n')
                .map(|(_, tone)| tone.split("\nPriority:").next().unwrap_or(tone))
                .unwrap_or(cleanup_preset),
        ),
        ("{{ formatting_rules }}", formatting_rules),
        ("{{ snippet_overrides }}", snippet_overrides),
        ("{{ evidence }}", evidence),
        ("{{ active_app }}", active_app),
    ];
    let mut rendered = String::with_capacity(template.len());
    let mut cursor = 0usize;
    loop {
        let next = placeholders
            .iter()
            .filter_map(|(token, value)| {
                template[cursor..]
                    .find(token)
                    .map(|offset| (cursor + offset, *token, *value))
            })
            .min_by_key(|(position, _, _)| *position);
        let Some((position, token, value)) = next else {
            rendered.push_str(&template[cursor..]);
            break;
        };
        rendered.push_str(&template[cursor..position]);
        rendered.push_str(value);
        cursor = position + token.len();
    }
    rendered
}

pub(super) fn collapse_blank_lines(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut newline_run = 0;
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\r' && characters.peek() == Some(&'\n') {
            continue;
        }
        if character == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                result.push(character);
            }
        } else {
            newline_run = 0;
            result.push(character);
        }
    }
    result.truncate(result.trim_end().len());
    result
}

pub fn cleanup_max_output_tokens(intensity: &str, input_text: &str) -> u32 {
    let input_words = count_words(input_text) as u32;
    match intensity {
        "none" => (input_words + 32).clamp(64, 512),
        "light" => (input_words * 2 + 32).clamp(96, 768),
        "high" => (input_words + 64).clamp(96, 768),
        _ => (input_words * 2 + 64).clamp(128, 1024),
    }
}

pub fn fusion_max_output_tokens(primary: &str, alternate: &str) -> u32 {
    let input_words = (count_words(primary) + count_words(alternate)) as u32;
    (input_words / 2 + 32).clamp(64, 512)
}
