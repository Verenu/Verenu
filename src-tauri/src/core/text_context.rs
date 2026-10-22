#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentenceContext {
    NewSentence,
    MidSentence,
    Unknown,
}

impl SentenceContext {
    #[allow(dead_code, clippy::wrong_self_convention)]
    pub fn as_str(self) -> &'static str {
        match self {
            SentenceContext::NewSentence => "new_sentence",
            SentenceContext::MidSentence => "mid_sentence",
            SentenceContext::Unknown => "unknown",
        }
    }

    #[cfg(test)]
    pub fn should_capitalize(self) -> bool {
        !matches!(self, SentenceContext::MidSentence)
    }

    #[cfg(test)]
    pub fn should_add_space(self) -> bool {
        matches!(self, SentenceContext::MidSentence)
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionPrefixClass {
    PlainWordStart,
    SoftPunctuationPrefix,
    HardSentenceTerminator,
    InvisibleOrAmbiguousPrefix,
}

#[cfg(test)]
impl InjectionPrefixClass {
    #[allow(dead_code, clippy::wrong_self_convention)]
    pub fn as_str(self) -> &'static str {
        match self {
            InjectionPrefixClass::PlainWordStart => "plain_word_start",
            InjectionPrefixClass::SoftPunctuationPrefix => "soft_punctuation_prefix",
            InjectionPrefixClass::HardSentenceTerminator => "hard_sentence_terminator",
            InjectionPrefixClass::InvisibleOrAmbiguousPrefix => "invisible_or_ambiguous_prefix",
        }
    }
}

#[cfg(test)]
fn is_sentence_ender(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '\n' | '\r')
}

#[cfg(test)]
fn trim_context_tail(context: &str) -> &str {
    context.trim_end_matches(|c: char| c.is_whitespace() && !is_sentence_ender(c))
}

#[cfg(test)]
pub fn classify_local_context(context: &str) -> SentenceContext {
    let trimmed = trim_context_tail(context);
    if trimmed.is_empty()
        || trimmed
            .chars()
            .next_back()
            .map(is_sentence_ender)
            .unwrap_or(true)
    {
        SentenceContext::NewSentence
    } else {
        SentenceContext::MidSentence
    }
}

pub(crate) fn is_invisible_prefix_char(ch: char) -> bool {
    ch.is_whitespace()
        || ch.is_control()
        || matches!(
            ch,
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}'
        )
}

fn is_context_tail_wrapper_char(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | '"'
            | '\''
            | '\u{201C}'
            | '\u{201D}'
            | '\u{2018}'
            | '\u{2019}'
            | '\u{00AB}'
            | '\u{00BB}'
            | '\u{2039}'
            | '\u{203A}'
            | '*'
            | '_'
            | '~'
            | '\u{0060}'
    )
}

#[cfg(test)]
fn is_soft_punctuation_prefix_char(ch: char) -> bool {
    matches!(
        ch,
        ',' | ';'
            | ':'
            | '-'
            | '–'
            | '—'
            | '/'
            | '\\'
            | ')'
            | ']'
            | '}'
            | '>'
            | '('
            | '['
            | '{'
            | '<'
            | '"'
            | '\''
            | '“'
            | '”'
            | '‘'
            | '’'
            | '«'
            | '»'
            | '‹'
            | '›'
            | '…'
    )
}

#[cfg(test)]
pub fn classify_leading_prefix(text: &str) -> InjectionPrefixClass {
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.peek().copied() {
        if is_invisible_prefix_char(ch) {
            chars.next();
        } else {
            break;
        }
    }

    let Some(first_visible) = chars.next() else {
        return InjectionPrefixClass::InvisibleOrAmbiguousPrefix;
    };

    if first_visible == '.' {
        if matches!(chars.peek().copied(), Some('.')) {
            return InjectionPrefixClass::SoftPunctuationPrefix;
        }
        return InjectionPrefixClass::HardSentenceTerminator;
    }
    if matches!(first_visible, '!' | '?') {
        return InjectionPrefixClass::HardSentenceTerminator;
    }
    if is_soft_punctuation_prefix_char(first_visible) {
        return InjectionPrefixClass::SoftPunctuationPrefix;
    }
    if first_visible.is_alphanumeric() {
        return InjectionPrefixClass::PlainWordStart;
    }

    InjectionPrefixClass::InvisibleOrAmbiguousPrefix
}

pub fn classify_context_tail(text: &str) -> SentenceContext {
    if text.is_empty() || at_sentence_boundary(text) {
        return SentenceContext::NewSentence;
    }
    for (idx, ch) in text.char_indices().rev() {
        if matches!(ch, '\n' | '\r') {
            return SentenceContext::NewSentence;
        }
        if is_invisible_prefix_char(ch) || is_context_tail_wrapper_char(ch) {
            continue;
        }
        if ch == '.' {
            return if period_is_nonterminal(&text[..idx + ch.len_utf8()]) {
                SentenceContext::MidSentence
            } else {
                SentenceContext::NewSentence
            };
        }
        if matches!(ch, '!' | '?') {
            return SentenceContext::NewSentence;
        }
        if ch.is_alphanumeric() || matches!(ch, ',' | ';' | ':' | '-' | '–' | '—' | '/' | '\\')
        {
            return SentenceContext::MidSentence;
        }
        return SentenceContext::Unknown;
    }

    SentenceContext::Unknown
}

#[cfg(test)]
fn transform_first_cased_char(text: &str, capitalize: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut transformed = false;

    for ch in text.chars() {
        if !transformed && (ch.is_lowercase() || ch.is_uppercase()) {
            if capitalize {
                out.extend(ch.to_uppercase());
            } else {
                out.extend(ch.to_lowercase());
            }
            transformed = true;
        } else {
            out.push(ch);
        }
    }

    if transformed {
        out
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
pub fn apply_contextual_casing(text: &str, context: SentenceContext) -> String {
    transform_first_cased_char(text, context.should_capitalize())
}

#[cfg(test)]
pub fn should_add_injection_space(context: SentenceContext, prefix: InjectionPrefixClass) -> bool {
    matches!(prefix, InjectionPrefixClass::PlainWordStart) && context.should_add_space()
}

#[cfg(test)]
fn is_opening_prefix_char(ch: char) -> bool {
    matches!(
        ch,
        '(' | '[' | '{' | '<' | '"' | '\'' | '“' | '‘' | '«' | '‹'
    )
}

#[cfg(test)]
fn first_visible_char(text: &str) -> Option<char> {
    text.chars().find(|ch| !is_invisible_prefix_char(*ch))
}

#[cfg(test)]
fn tail_ends_with_visible_char(text: &str) -> bool {
    text.chars()
        .next_back()
        .map(|ch| !is_invisible_prefix_char(ch))
        .unwrap_or(false)
}

#[cfg(test)]
pub fn should_add_leading_injection_space(
    text: &str,
    context: SentenceContext,
    prefix: InjectionPrefixClass,
    source_allows_spacing: bool,
    context_tail: &str,
) -> bool {
    if text.starts_with(char::is_whitespace) {
        return false;
    }
    if !source_allows_spacing {
        return false;
    }
    if !tail_ends_with_visible_char(context_tail) {
        return false;
    }

    match prefix {
        InjectionPrefixClass::PlainWordStart => {
            matches!(
                context,
                SentenceContext::MidSentence | SentenceContext::NewSentence
            )
        }
        InjectionPrefixClass::SoftPunctuationPrefix => {
            matches!(first_visible_char(text), Some(ch) if is_opening_prefix_char(ch))
        }
        InjectionPrefixClass::HardSentenceTerminator
        | InjectionPrefixClass::InvisibleOrAmbiguousPrefix => false,
    }
}

#[cfg(test)]
pub fn should_capitalize_injection(context: SentenceContext, prefix: InjectionPrefixClass) -> bool {
    match prefix {
        InjectionPrefixClass::PlainWordStart => context.should_capitalize(),
        InjectionPrefixClass::HardSentenceTerminator => true,
        InjectionPrefixClass::SoftPunctuationPrefix
        | InjectionPrefixClass::InvisibleOrAmbiguousPrefix => false,
    }
}

#[cfg(test)]
pub fn format_injection_text(
    text: &str,
    context: SentenceContext,
    prefix: InjectionPrefixClass,
) -> String {
    match prefix {
        InjectionPrefixClass::PlainWordStart => apply_contextual_casing(text, context),
        InjectionPrefixClass::HardSentenceTerminator => transform_first_cased_char(text, true),
        InjectionPrefixClass::SoftPunctuationPrefix
        | InjectionPrefixClass::InvisibleOrAmbiguousPrefix => text.to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseAction {
    Preserve,
    CapitalizeFirstWord,
    LowercaseFirstWord,
}

impl CaseAction {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preserve => "preserve",
            Self::CapitalizeFirstWord => "capitalize_first_word",
            Self::LowercaseFirstWord => "lowercase_first_word",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertionDecision {
    pub text: String,
    pub case_action: CaseAction,
    pub leading_space: bool,
    pub trailing_space: bool,
    pub reason: &'static str,
}

pub struct CaretTextContext<'a> {
    pub left: &'a str,
    pub right: &'a str,
    pub left_reliable: bool,
    pub right_reliable: bool,
    pub language: &'a str,
    pub casing_enabled: bool,
    pub preserve_sentence_case: bool,
    pub protected_initial_case: bool,
}

const PROTECTED_TITLECASE_WORDS: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

fn language_uses_interword_spaces(language: &str) -> bool {
    let primary = language.split(['-', '_']).next().unwrap_or(language);
    !matches!(primary, "zh" | "ja" | "th" | "lo" | "km")
}

fn language_uses_english_case_policy(language: &str) -> bool {
    let primary = language.split(['-', '_']).next().unwrap_or(language);
    matches!(primary, "" | "auto" | "en")
}

fn is_sentence_terminator(ch: char) -> bool {
    matches!(
        ch,
        '.' | '!' | '?' | '…' | '。' | '！' | '？' | '؟' | '۔' | '‼' | '⁉'
    )
}

fn is_terminal_wrapper(ch: char) -> bool {
    is_closing_delimiter(ch) || matches!(ch, '*' | '_' | '~' | '`')
}

fn has_unclosed_quote(chars: impl Iterator<Item = char>, quote: char) -> bool {
    let mut chars = chars.peekable();
    let mut previous = None;
    let mut escaped = false;
    let mut open = false;
    while let Some(ch) = chars.next() {
        if ch == quote
            && !escaped
            && !(quote == '\''
                && previous.is_some_and(char::is_alphanumeric)
                && chars.peek().is_some_and(|ch| ch.is_alphanumeric()))
        {
            open = !open;
        }
        escaped = ch == '\\' && !escaped;
        previous = Some(ch);
    }
    open
}

fn ends_with_opening_delimiter(text: &str) -> bool {
    let Some(last) = text.chars().next_back() else {
        return false;
    };
    if matches!(last, '\'' | '"') {
        let prefix = &text[..text.len() - last.len_utf8()];
        return !has_unclosed_quote(prefix.chars(), last)
            && prefix.chars().next_back().is_none_or(|ch| {
                ch.is_whitespace() || is_opening_delimiter(ch) || matches!(ch, ':' | ',')
            });
    }
    is_opening_delimiter(last)
}

fn is_opening_delimiter(ch: char) -> bool {
    matches!(
        ch,
        '(' | '[' | '{' | '<' | '\'' | '"' | '‘' | '“' | '«' | '‹'
    )
}

fn is_closing_delimiter(ch: char) -> bool {
    matches!(
        ch,
        ')' | ']' | '}' | '>' | '\'' | '"' | '’' | '”' | '»' | '›'
    )
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
    )
}

fn is_token_joiner(ch: char) -> bool {
    matches!(ch, '/' | '\\' | '_' | '@' | '#' | '-')
}

fn is_email_local_part_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '.' | '_' | '%' | '+' | '-')
}

fn starts_with_email_domain_fragment(text: &str) -> bool {
    let Some(candidate) = text.strip_prefix('@') else {
        return false;
    };
    let Some(candidate) = candidate.split_whitespace().next() else {
        return false;
    };
    let domain = candidate.trim_end_matches(|ch: char| {
        is_closing_delimiter(ch) || is_sentence_terminator(ch) || matches!(ch, ',' | ';' | ':')
    });
    let mut labels = domain.split('.');
    let Some(first) = labels.next() else {
        return false;
    };
    let valid_label = |label: &str| {
        !label.is_empty()
            && label.chars().next().is_some_and(char::is_alphanumeric)
            && label.chars().next_back().is_some_and(char::is_alphanumeric)
            && label.chars().all(|ch| ch.is_alphanumeric() || ch == '-')
    };
    valid_label(first) && labels.next().is_some_and(valid_label) && labels.all(valid_label)
}

fn is_email_domain_continuation(left: &str, text: &str) -> bool {
    let Some(local_part) = left.split_whitespace().next_back() else {
        return false;
    };
    !local_part.is_empty()
        && local_part
            .chars()
            .all(|ch| is_email_local_part_char(ch) && !ch.is_uppercase())
        && starts_with_email_domain_fragment(text)
}

fn is_url_or_path_prefix(left: &str) -> bool {
    let Some(token) = left.split_whitespace().next_back() else {
        return false;
    };
    let token = token
        .trim_start_matches(is_opening_delimiter)
        .trim_end_matches(is_terminal_wrapper);
    let has_path_marker = token.starts_with('/') || token.contains('/');
    has_path_marker
        && token.chars().all(|ch| {
            ch.is_alphanumeric()
                || matches!(
                    ch,
                    '/' | '\\' | '.' | '_' | '-' | '+' | '~' | '%' | ':' | '?' | '&' | '=' | '#'
                )
        })
}

fn is_punctuation(ch: char) -> bool {
    is_sentence_terminator(ch)
        || is_opening_delimiter(ch)
        || is_closing_delimiter(ch)
        || is_token_joiner(ch)
        || matches!(ch, ',' | ';' | ':' | '，' | '；' | '：' | '…')
}

fn starts_with_contraction_suffix(text: &str) -> bool {
    const SUFFIXES: &[&str] = &[
        "'s", "’s", "'re", "’re", "'ve", "’ve", "'ll", "’ll", "'d", "’d", "'m", "’m", "n't", "n’t",
    ];
    SUFFIXES.iter().any(|suffix| {
        let mut lowercase = text.chars().flat_map(char::to_lowercase);
        suffix.chars().all(|ch| lowercase.next() == Some(ch))
            && lowercase.next().is_none_or(|ch| !ch.is_alphanumeric())
    })
}

fn first_word_span(text: &str) -> Option<(usize, usize)> {
    let mut start = None;
    let mut end = 0;
    for (idx, ch) in text.char_indices() {
        if start.is_none() {
            if ch.is_alphabetic() {
                start = Some(idx);
                end = idx + ch.len_utf8();
            }
        } else if ch.is_alphabetic() || matches!(ch, '\'' | '’') {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    start.map(|value| (value, end))
}

fn word_is_all_lowercase(word: &str) -> bool {
    let mut cased = word.chars().filter(|ch| ch.is_alphabetic());
    cased.next().is_some_and(|ch| !ch.is_uppercase()) && cased.all(|ch| !ch.is_uppercase())
}

fn word_is_simple_titlecase(word: &str) -> bool {
    let mut cased = word.chars().filter(|ch| ch.is_alphabetic());
    cased.next().is_some_and(|ch| ch.is_uppercase()) && cased.all(|ch| !ch.is_uppercase())
}

fn is_first_person_pronoun(word: &str) -> bool {
    matches!(
        word,
        "I" | "I'm" | "I’m" | "I'll" | "I’ll" | "I've" | "I’ve" | "I'd" | "I’d"
    )
}

fn next_word_is_titlecase(text: &str, first_word_end: usize) -> bool {
    first_word_span(&text[first_word_end..])
        .map(|(start, end)| &text[first_word_end + start..first_word_end + end])
        .is_some_and(|word| word_is_simple_titlecase(word) && !is_first_person_pronoun(word))
}

fn should_lowercase_continuation(
    text: &str,
    start: usize,
    end: usize,
    follows_soft_punctuation: bool,
) -> bool {
    let word = &text[start..end];
    if !word_is_simple_titlecase(word) || is_first_person_pronoun(word) {
        return false;
    }
    let lowercase = word.to_lowercase();
    !PROTECTED_TITLECASE_WORDS.contains(&lowercase.as_str())
        && (follows_soft_punctuation || !next_word_is_titlecase(text, end))
}

fn ends_with_soft_continuation_punctuation(text: &str) -> bool {
    text.trim_end_matches(char::is_whitespace)
        .chars()
        .next_back()
        .is_some_and(|ch| matches!(ch, ',' | ';' | ':' | '，' | '；' | '：'))
}

fn transform_word_first_char(text: &str, start: usize, uppercase: bool) -> String {
    let Some(first) = text[start..].chars().next() else {
        return text.to_owned();
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..start]);
    if uppercase {
        out.extend(first.to_uppercase());
    } else {
        out.extend(first.to_lowercase());
    }
    out.push_str(&text[start + first.len_utf8()..]);
    out
}

fn starts_with_literal_token(text: &str) -> bool {
    let token = text
        .trim_start_matches(|ch: char| ch.is_whitespace() || is_opening_delimiter(ch))
        .split_whitespace()
        .next()
        .unwrap_or_default();
    let bare = token.trim_end_matches(|ch| is_closing_delimiter(ch) || is_punctuation(ch));
    if bare.eq_ignore_ascii_case("and/or") || bare.eq_ignore_ascii_case("either/or") {
        return false;
    }
    let dotted_literal = !bare.eq_ignore_ascii_case("e.g")
        && !bare.eq_ignore_ascii_case("i.e")
        && bare.contains('.')
        && bare.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|ch| ch.is_alphanumeric() || ch == '-')
        });
    token.contains(['@', '/', '\\', '_', '`', '='])
        || token.starts_with(['#', '$'])
        || (token.starts_with('-') && token.chars().any(char::is_alphabetic))
        || token
            .strip_prefix('.')
            .is_some_and(|rest| rest.chars().next().is_some_and(char::is_alphanumeric))
        || dotted_literal
}

fn starts_with_quoted_phrase(text: &str) -> bool {
    let Some(quote @ ('\'' | '"')) = text.chars().next() else {
        return false;
    };
    let rest = &text[quote.len_utf8()..];
    rest.chars()
        .next()
        .is_some_and(|ch| ch.is_alphanumeric() || is_opening_delimiter(ch))
        && rest.contains(quote)
        && !starts_with_contraction_suffix(text)
}

fn line_is_list_prefix(line: &str) -> bool {
    let trimmed = line.trim();
    if matches!(trimmed, "-" | "*" | "+" | "•" | "[ ]" | "[x]" | "[X]") {
        return true;
    }
    let numbered = trimmed.strip_suffix(['.', ')']).unwrap_or_default().trim();
    !numbered.is_empty() && numbered.chars().all(|ch| ch.is_ascii_digit())
}

fn period_is_nonterminal(text: &str) -> bool {
    let Some(before_period) = text.strip_suffix('.') else {
        return false;
    };
    let token = before_period
        .split_whitespace()
        .next_back()
        .unwrap_or_default()
        .trim_matches(is_opening_delimiter);
    let lowercase = token.to_lowercase();
    const ABBREVIATIONS: &[&str] = &[
        "mr", "mrs", "ms", "mx", "dr", "prof", "sr", "jr", "st", "vs", "e.g", "i.e", "no", "fig",
        "dept", "inc", "ltd", "co", "ph.d", "b.sc", "m.sc", "d.phil", "ed.d",
    ];
    if ABBREVIATIONS.contains(&lowercase.as_str()) {
        return true;
    }
    if token.chars().count() == 1 && token.chars().all(char::is_alphabetic) {
        return true;
    }
    token.contains('.')
        && token.split('.').any(|part| part.chars().count() == 1)
        && token.split('.').all(|part| {
            let length = part.chars().count();
            (1..=3).contains(&length)
                && part.chars().all(char::is_alphabetic)
                && (length == 1 || part.chars().next().is_some_and(char::is_uppercase))
        })
}

fn at_sentence_boundary(left: &str) -> bool {
    if left.is_empty() || left.ends_with(['\n', '\r']) {
        return true;
    }
    let line = left.rsplit(['\n', '\r']).next().unwrap_or_default();
    if line.trim().is_empty() || line_is_list_prefix(line) {
        return true;
    }

    let trimmed = left.trim_end_matches(char::is_whitespace);
    if let Some(last) = trimmed.chars().next_back() {
        if ends_with_opening_delimiter(trimmed) {
            let prefix = &trimmed[..trimmed.len() - last.len_utf8()];
            let prefix = prefix.trim_end_matches(char::is_whitespace);
            if prefix.is_empty()
                || prefix.ends_with(['\n', '\r'])
                || prefix
                    .chars()
                    .next_back()
                    .is_some_and(|ch| is_sentence_terminator(ch) || matches!(ch, ':' | ','))
            {
                return true;
            }
        }
    }

    let without_closers = trimmed.trim_end_matches(is_terminal_wrapper);
    match without_closers.chars().next_back() {
        None => true,
        Some('.') => !period_is_nonterminal(without_closers),
        Some(ch) if is_sentence_terminator(ch) => true,
        Some(_) => false,
    }
}

pub fn decide_insertion(text: &str, context: CaretTextContext<'_>) -> InsertionDecision {
    if text.is_empty() {
        return InsertionDecision {
            text: text.to_owned(),
            case_action: CaseAction::Preserve,
            leading_space: false,
            trailing_space: false,
            reason: "empty_payload",
        };
    }

    let first_word = first_word_span(text);
    let boundary = context.left_reliable.then(|| {
        if let Some((start, _)) = first_word {
            let prefix = &text[..start];
            if prefix.contains(['\n', '\r']) || prefix.chars().any(is_sentence_terminator) {
                return at_sentence_boundary(&format!("{}{prefix}", context.left));
            }
        }
        at_sentence_boundary(context.left)
    });
    let (mut adjusted, case_action) = if !context.casing_enabled
        || context.protected_initial_case
        || starts_with_literal_token(text)
    {
        (text.to_owned(), CaseAction::Preserve)
    } else {
        match first_word {
            Some((start, end)) if boundary == Some(true) && !context.preserve_sentence_case => {
                let word = &text[start..end];
                if word_is_all_lowercase(word) {
                    (
                        transform_word_first_char(text, start, true),
                        CaseAction::CapitalizeFirstWord,
                    )
                } else {
                    (text.to_owned(), CaseAction::Preserve)
                }
            }
            Some((start, end))
                if boundary == Some(false)
                    && language_uses_english_case_policy(context.language) =>
            {
                if should_lowercase_continuation(
                    text,
                    start,
                    end,
                    ends_with_soft_continuation_punctuation(context.left),
                ) {
                    (
                        transform_word_first_char(text, start, false),
                        CaseAction::LowercaseFirstWord,
                    )
                } else {
                    (text.to_owned(), CaseAction::Preserve)
                }
            }
            _ => (text.to_owned(), CaseAction::Preserve),
        }
    };

    let first = text.chars().find(|ch| !ch.is_whitespace());
    let last = text.chars().rev().find(|ch| !ch.is_whitespace());
    let left_immediate = context.left.chars().next_back();
    let right_immediate = context.right.chars().next();
    let auto_detected_no_space_script = matches!(context.language, "auto" | "")
        && left_immediate
            .filter(|_| context.left_reliable)
            .into_iter()
            .chain(right_immediate.filter(|_| context.right_reliable))
            .chain(first)
            .chain(last)
            .any(is_cjk);
    let uses_spaces =
        language_uses_interword_spaces(context.language) && !auto_detected_no_space_script;
    let terminal_markup = context.left_reliable
        && left_immediate.is_some_and(|ch| matches!(ch, '_' | '*' | '~' | '`'))
        && context
            .left
            .trim_end_matches(is_terminal_wrapper)
            .chars()
            .next_back()
            .is_some_and(is_sentence_terminator);
    let token_local =
        (context.left_reliable && left_immediate.is_some_and(is_token_joiner) && !terminal_markup)
            || (context.right_reliable && right_immediate.is_some_and(is_token_joiner));

    let leading_space = context.left_reliable
        && uses_spaces
        && !token_local
        && !starts_with_contraction_suffix(text)
        && !text.starts_with(char::is_whitespace)
        && left_immediate.is_some_and(|ch| {
            !ch.is_whitespace()
                && !ends_with_opening_delimiter(context.left)
                && (!is_token_joiner(ch) || terminal_markup)
        })
        && first.is_some_and(|ch| {
            (!is_closing_delimiter(ch) || is_opening_delimiter(ch))
                && (!is_token_joiner(ch)
                    || (ch == '@' && !is_email_domain_continuation(context.left, text))
                    || (ch == '#' && !is_url_or_path_prefix(context.left))
                    || (ch == '-' && starts_with_literal_token(text)))
                && !is_sentence_terminator(ch)
                && !matches!(ch, ',' | ';' | ':' | '，' | '；' | '：')
        });
    let trailing_space = context.right_reliable
        && uses_spaces
        && !token_local
        && !text.ends_with(char::is_whitespace)
        && right_immediate.is_some_and(|ch| {
            !ch.is_whitespace()
                && (!is_closing_delimiter(ch)
                    || (starts_with_quoted_phrase(context.right)
                        && (!context.left_reliable
                            || !has_unclosed_quote(context.left.chars().chain(text.chars()), ch))))
                && !is_token_joiner(ch)
                && (!is_punctuation(ch) || is_opening_delimiter(ch))
        })
        && last.is_some_and(|ch| !ends_with_opening_delimiter(text) && !is_token_joiner(ch));

    if leading_space {
        adjusted.insert(0, ' ');
    }
    if trailing_space {
        adjusted.push(' ');
    }

    InsertionDecision {
        text: adjusted,
        case_action,
        leading_space,
        trailing_space,
        reason: match (context.left_reliable, context.right_reliable, boundary) {
            (false, false, _) => "unreliable_context",
            (false, true, _) => "right_context_only",
            (true, false, _) => "left_context_only",
            (true, true, Some(true)) => "sentence_boundary",
            (true, true, Some(false)) => "continuation",
            (true, true, None) => "unknown_context",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("text_context_cases.rs");

    #[test]
    fn insertion_edge_case_corpus() {
        for (text, left, right, expected) in insertion_cases() {
            assert_eq!(
                smart(&text, &left, &right).text,
                expected,
                "left={left:?}, text={text:?}, right={right:?}"
            );
        }
    }

    #[test]
    fn unreliable_edges_cannot_influence_formatting() {
        for left_reliable in [false, true] {
            for right_reliable in [false, true] {
                for ignored in ["", "中", "/", "\n", "Done.", "\"", "_"] {
                    let decide = |left, right| {
                        decide_insertion(
                            "Hello",
                            CaretTextContext {
                                left,
                                right,
                                left_reliable,
                                right_reliable,
                                language: "auto",
                                casing_enabled: true,
                                preserve_sentence_case: false,
                                protected_initial_case: false,
                            },
                        )
                        .text
                    };
                    assert_eq!(
                        decide(
                            if left_reliable { "word" } else { ignored },
                            if right_reliable { "word" } else { ignored }
                        ),
                        decide(
                            if left_reliable { "word" } else { "" },
                            if right_reliable { "word" } else { "" }
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn formatting_is_idempotent_and_keeps_protected_case() {
        for (text, left, right, _) in insertion_cases() {
            let once = smart(&text, &left, &right).text;
            assert_eq!(smart(&once, &left, &right).text, once);
            for (casing_enabled, preserve_sentence_case, protected_initial_case) in
                [(false, false, false), (true, false, true)]
            {
                let decision = decide_insertion(
                    &text,
                    CaretTextContext {
                        left: &left,
                        right: &right,
                        left_reliable: true,
                        right_reliable: true,
                        language: "en",
                        casing_enabled,
                        preserve_sentence_case,
                        protected_initial_case,
                    },
                );
                assert_eq!(decision.case_action, CaseAction::Preserve);
                assert_eq!(decision.text.trim(), text.trim());
            }
        }
    }

    #[test]
    fn contraction_prefix_matches_whole_string_lowercasing() {
        const SUFFIXES: &[&str] = &[
            "'s", "’s", "'re", "’re", "'ve", "’ve", "'ll", "’ll", "'d", "’d", "'m", "’m", "n't",
            "n’t",
        ];
        for prefix in SUFFIXES.iter().copied().chain(["", "n", "'", "word", "İ"]) {
            for prefix in [prefix.to_owned(), prefix.to_uppercase()] {
                for tail in ["", " ", ".", "x", "1", "界", "İ", "Σ", "\u{301}"] {
                    let text = format!("{prefix}{tail}");
                    let lowercase = text.to_lowercase();
                    let expected = SUFFIXES.iter().any(|suffix| {
                        lowercase.strip_prefix(suffix).is_some_and(|rest| {
                            rest.chars().next().is_none_or(|ch| !ch.is_alphanumeric())
                        })
                    });
                    assert_eq!(starts_with_contraction_suffix(&text), expected, "{text:?}");
                }
            }
        }
    }

    #[test]
    fn lowercase_word_check_preserves_uncased_letters_and_empty_words() {
        for word in ["", "123", "'’"] {
            assert!(!word_is_all_lowercase(word));
        }
        for word in ["hello", "éclair", "界", "a'界", "e\u{301}"] {
            assert!(word_is_all_lowercase(word));
        }
        for word in ["Hello", "Éclair", "aB", "界A"] {
            assert!(!word_is_all_lowercase(word));
        }
    }

    fn smart(text: &str, left: &str, right: &str) -> InsertionDecision {
        decide_insertion(
            text,
            CaretTextContext {
                left,
                right,
                left_reliable: true,
                right_reliable: true,
                language: "en",
                casing_enabled: true,
                preserve_sentence_case: false,
                protected_initial_case: false,
            },
        )
    }

    #[test]
    fn smart_formatting_handles_both_caret_edges() {
        assert_eq!(smart("next", "Hello.", "").text, " Next");
        assert_eq!(smart("The answer", "I think", "").text, " the answer");
        assert_eq!(smart("new", "Alpha", "beta").text, " new ");
        assert_eq!(smart("value", "call(", ")").text, "value");
        assert_eq!(smart("value", "path/", "file").text, "value");
        assert_eq!(smart("domain", "name@", ".com").text, "domain");
        assert_eq!(smart("'s ready", "it", "").text, "'s ready");
        assert_eq!(smart("n't ready", "is", "").text, "n't ready");
        assert_eq!(smart(", however", "word", "").text, ", however");
        assert_eq!(smart("\"hello\"", "They said", "").text, " \"hello\"");
    }

    #[test]
    fn wrapped_terminal_t3_skill_token_keeps_canonical_case_at_sentence_boundary() {
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
        let normalized = catalog.normalize_mentions("($BABYSIT-PR)");
        assert_eq!(normalized, "( $babysit-pr ) ");
        assert!(catalog.starts_with_skill_mention(&normalized));

        let protected = decide_insertion(
            &normalized,
            CaretTextContext {
                left: "Previous sentence.",
                right: "",
                left_reliable: true,
                right_reliable: true,
                language: "en",
                casing_enabled: true,
                preserve_sentence_case: false,
                protected_initial_case: catalog.starts_with_skill_mention(&normalized),
            },
        );
        assert_eq!(protected.text, " ( $babysit-pr ) ");
        assert_eq!(protected.case_action, CaseAction::Preserve);

        let ordinary = decide_insertion(
            "use a skill",
            CaretTextContext {
                left: "Previous sentence.",
                right: "",
                left_reliable: true,
                right_reliable: true,
                language: "en",
                casing_enabled: true,
                preserve_sentence_case: false,
                protected_initial_case: catalog.starts_with_skill_mention("use a skill"),
            },
        );
        assert_eq!(ordinary.text, " Use a skill");
        assert_eq!(ordinary.case_action, CaseAction::CapitalizeFirstWord);
    }

    #[test]
    fn unfinished_sentence_forces_ordinary_initial_capital_lowercase() {
        assert_eq!(smart("Hello again", "unfinished", "").text, " hello again");
        assert_eq!(
            smart("Please add this", "unfinished ", "").text,
            "please add this"
        );
        assert_eq!(
            smart("Actually yes", "unfinished,", "").text,
            " actually yes"
        );
        assert_eq!(
            smart("Dabba Doo.", "Yabba Dabba Dooba,", "").text,
            " dabba Doo."
        );
        assert_eq!(smart("I agree", "unfinished", "").text, " I agree");
        assert_eq!(smart("I'm ready", "unfinished", "").text, " I'm ready");
        assert_eq!(
            smart("When I return", "unfinished", "").text,
            " when I return"
        );
        assert_eq!(
            smart("New York works", "unfinished", "").text,
            " New York works"
        );

        let protected = decide_insertion(
            "Verenu works",
            CaretTextContext {
                left: "unfinished",
                right: "",
                left_reliable: true,
                right_reliable: true,
                language: "en",
                casing_enabled: true,
                preserve_sentence_case: false,
                protected_initial_case: true,
            },
        );
        assert_eq!(protected.text, " Verenu works");
    }

    #[test]
    fn sentence_boundary_matrix_handles_abbreviations_quotes_and_lists() {
        assert_eq!(smart("Next", "Done.", "").text, " Next");
        assert_eq!(smart("Next", "Ask Dr.", "").text, " next");
        assert_eq!(smart("Next", "Use e.g.", "").text, " next");
        assert_eq!(smart("hello", "They said, \"", "").text, "Hello");
        assert_eq!(smart("next item", "Items:\n- ", "").text, "Next item");
        assert_eq!(smart("next item", "Items:\n2. ", "").text, "Next item");
        assert_eq!(
            classify_context_tail("Ask Dr."),
            SentenceContext::MidSentence
        );
        assert_eq!(
            classify_context_tail("They said, \""),
            SentenceContext::NewSentence
        );
    }

    #[test]
    fn one_sided_context_only_changes_the_dependent_edge() {
        let left_only = decide_insertion(
            "Hello",
            CaretTextContext {
                left: "unfinished",
                right: "unknown",
                left_reliable: true,
                right_reliable: false,
                language: "en",
                casing_enabled: true,
                preserve_sentence_case: false,
                protected_initial_case: false,
            },
        );
        assert_eq!(left_only.text, " hello");
        assert!(left_only.leading_space);
        assert!(!left_only.trailing_space);

        let right_only = decide_insertion(
            "Hello",
            CaretTextContext {
                left: "unknown",
                right: "world",
                left_reliable: false,
                right_reliable: true,
                language: "en",
                casing_enabled: true,
                preserve_sentence_case: false,
                protected_initial_case: false,
            },
        );
        assert_eq!(right_only.text, "Hello ");
        assert!(!right_only.leading_space);
        assert!(right_only.trailing_space);
    }

    #[test]
    fn smart_formatting_preserves_case_that_is_not_provably_safe() {
        assert_eq!(smart("THE answer", "I think", "").text, " THE answer");
        assert_eq!(smart("iPhone works", "Done.", "").text, " iPhone works");
        assert_eq!(smart("May arrive", "They", "").text, " May arrive");
        assert_eq!(smart("NASA agrees", "They", "").text, " NASA agrees");
        let non_english = decide_insertion(
            "The answer",
            CaretTextContext {
                left: "Selon",
                right: "",
                left_reliable: true,
                right_reliable: true,
                language: "fr",
                casing_enabled: true,
                preserve_sentence_case: false,
                protected_initial_case: false,
            },
        );
        assert_eq!(non_english.text, " The answer");
    }

    #[test]
    fn smart_formatting_preserves_unknown_context_exactly() {
        let decision = decide_insertion(
            "The answer",
            CaretTextContext {
                left: "ignored",
                right: "ignored",
                left_reliable: false,
                right_reliable: false,
                language: "en",
                casing_enabled: true,
                preserve_sentence_case: false,
                protected_initial_case: false,
            },
        );
        assert_eq!(decision.text, "The answer");
        assert!(!decision.leading_space);
        assert!(!decision.trailing_space);
    }

    #[test]
    fn smart_formatting_respects_newlines_and_no_space_languages() {
        assert_eq!(smart("next", "Hello\n", "").text, "Next");
        let decision = decide_insertion(
            "世界",
            CaretTextContext {
                left: "你好。",
                right: "朋友",
                left_reliable: true,
                right_reliable: true,
                language: "zh-CN",
                casing_enabled: true,
                preserve_sentence_case: false,
                protected_initial_case: false,
            },
        );
        assert_eq!(decision.text, "世界");
        let auto = decide_insertion(
            "世界",
            CaretTextContext {
                left: "你好",
                right: "朋友",
                left_reliable: true,
                right_reliable: true,
                language: "auto",
                casing_enabled: true,
                preserve_sentence_case: false,
                protected_initial_case: false,
            },
        );
        assert_eq!(auto.text, "世界");
    }

    #[test]
    fn blank_context_capitalizes() {
        assert_eq!(classify_local_context(""), SentenceContext::NewSentence);
        assert!(SentenceContext::NewSentence.should_capitalize());
        assert!(!SentenceContext::NewSentence.should_add_space());
    }

    #[test]
    fn unknown_context_capitalizes() {
        assert!(SentenceContext::Unknown.should_capitalize());
        assert!(!SentenceContext::Unknown.should_add_space());
    }

    #[test]
    fn sentence_enders_capitalize() {
        assert_eq!(
            classify_local_context("hello."),
            SentenceContext::NewSentence
        );
        assert_eq!(
            classify_local_context("hello?"),
            SentenceContext::NewSentence
        );
        assert_eq!(
            classify_local_context("hello!"),
            SentenceContext::NewSentence
        );
        assert_eq!(
            classify_local_context("hello.\n"),
            SentenceContext::NewSentence
        );
    }

    #[test]
    fn mid_sentence_context_lowercases() {
        assert_eq!(
            classify_local_context("hello"),
            SentenceContext::MidSentence
        );
        assert_eq!(
            classify_local_context("hello,"),
            SentenceContext::MidSentence
        );
        assert_eq!(
            classify_local_context("hello /"),
            SentenceContext::MidSentence
        );
    }

    #[test]
    fn context_tail_ignores_trailing_spaces_and_wrappers() {
        assert_eq!(classify_context_tail("hi "), SentenceContext::MidSentence);
        assert_eq!(classify_context_tail("hi\""), SentenceContext::MidSentence);
        assert_eq!(classify_context_tail("hi)"), SentenceContext::MidSentence);
        assert_eq!(classify_context_tail("hi*"), SentenceContext::MidSentence);
        assert_eq!(classify_context_tail("hi_"), SentenceContext::MidSentence);
        assert_eq!(classify_context_tail("hi`"), SentenceContext::MidSentence);
        assert_eq!(classify_context_tail("hi.\""), SentenceContext::NewSentence);
        assert_eq!(classify_context_tail("hi.*"), SentenceContext::NewSentence);
        assert_eq!(
            classify_context_tail("hi.\u{200B} "),
            SentenceContext::NewSentence
        );
        assert_eq!(
            classify_context_tail("\u{200B}  "),
            SentenceContext::Unknown
        );
    }

    #[test]
    fn context_tail_newline_is_sentence_boundary() {
        assert_eq!(
            classify_context_tail("hello\n"),
            SentenceContext::NewSentence
        );
        assert_eq!(
            classify_context_tail("hello\r\n"),
            SentenceContext::NewSentence
        );
        assert_eq!(
            classify_context_tail("hello,\n"),
            SentenceContext::NewSentence
        );
    }

    #[test]
    fn leading_prefix_classifier_finds_plain_words() {
        assert_eq!(
            classify_leading_prefix("hello"),
            InjectionPrefixClass::PlainWordStart
        );
        assert_eq!(
            classify_leading_prefix("  hello"),
            InjectionPrefixClass::PlainWordStart
        );
    }

    #[test]
    fn leading_prefix_classifier_finds_soft_punctuation() {
        assert_eq!(
            classify_leading_prefix(", hello"),
            InjectionPrefixClass::SoftPunctuationPrefix
        );
        assert_eq!(
            classify_leading_prefix("\"hello\""),
            InjectionPrefixClass::SoftPunctuationPrefix
        );
        assert_eq!(
            classify_leading_prefix("(hello"),
            InjectionPrefixClass::SoftPunctuationPrefix
        );
        assert_eq!(
            classify_leading_prefix("...hello"),
            InjectionPrefixClass::SoftPunctuationPrefix
        );
        assert_eq!(
            classify_leading_prefix("…hello"),
            InjectionPrefixClass::SoftPunctuationPrefix
        );
    }

    #[test]
    fn leading_prefix_classifier_finds_sentence_terminators() {
        assert_eq!(
            classify_leading_prefix("!hello"),
            InjectionPrefixClass::HardSentenceTerminator
        );
        assert_eq!(
            classify_leading_prefix("?hello"),
            InjectionPrefixClass::HardSentenceTerminator
        );
        assert_eq!(
            classify_leading_prefix(".hello"),
            InjectionPrefixClass::HardSentenceTerminator
        );
    }

    #[test]
    fn leading_prefix_classifier_treats_invisible_only_input_as_ambiguous() {
        assert_eq!(
            classify_leading_prefix(""),
            InjectionPrefixClass::InvisibleOrAmbiguousPrefix
        );
        assert_eq!(
            classify_leading_prefix("\u{200B}\u{FEFF}"),
            InjectionPrefixClass::InvisibleOrAmbiguousPrefix
        );
    }

    #[test]
    fn trailing_whitespace_after_sentence_end_keeps_capitalization() {
        assert_eq!(
            classify_local_context("hello. "),
            SentenceContext::NewSentence
        );
        assert_eq!(
            classify_local_context("hello?\t"),
            SentenceContext::NewSentence
        );
    }

    #[test]
    fn spacing_follows_sentence_context() {
        assert!(!SentenceContext::NewSentence.should_add_space());
        assert!(SentenceContext::MidSentence.should_add_space());
        assert!(!SentenceContext::Unknown.should_add_space());
    }

    #[test]
    fn contextual_casing_capitalizes_sentence_starts() {
        assert_eq!(
            apply_contextual_casing("hello world", SentenceContext::NewSentence),
            "Hello world"
        );
        assert_eq!(
            apply_contextual_casing("\"hello world\"", SentenceContext::NewSentence),
            "\"Hello world\""
        );
        assert_eq!(
            apply_contextual_casing("123 hello", SentenceContext::NewSentence),
            "123 Hello"
        );
    }

    #[test]
    fn contextual_casing_lowercases_mid_sentence_starts() {
        assert_eq!(
            apply_contextual_casing("Hello world", SentenceContext::MidSentence),
            "hello world"
        );
        assert_eq!(
            apply_contextual_casing("\"Hello world\"", SentenceContext::MidSentence),
            "\"hello world\""
        );
        assert_eq!(
            apply_contextual_casing("123 Hello", SentenceContext::MidSentence),
            "123 hello"
        );
    }

    #[test]
    fn punctuation_prefixes_keep_model_casing_and_skip_extra_spacing() {
        let prefix = classify_leading_prefix("\"Hello\"");
        assert_eq!(prefix, InjectionPrefixClass::SoftPunctuationPrefix);
        assert_eq!(
            format_injection_text("\"Hello\"", SentenceContext::MidSentence, prefix),
            "\"Hello\""
        );
        assert!(!should_add_injection_space(
            SentenceContext::MidSentence,
            prefix
        ));
        assert!(!should_capitalize_injection(
            SentenceContext::MidSentence,
            prefix
        ));
    }

    #[test]
    fn sentence_boundary_adds_space_before_plain_words() {
        assert!(should_add_leading_injection_space(
            "Hello world",
            SentenceContext::NewSentence,
            InjectionPrefixClass::PlainWordStart,
            true,
            "hello.",
        ));
        assert!(should_add_leading_injection_space(
            "Hello world",
            SentenceContext::MidSentence,
            InjectionPrefixClass::PlainWordStart,
            true,
            "hello",
        ));
        assert!(!should_add_leading_injection_space(
            "Hello world",
            SentenceContext::NewSentence,
            InjectionPrefixClass::PlainWordStart,
            false,
            "hello.",
        ));
        assert!(!should_add_leading_injection_space(
            "Hello world",
            SentenceContext::NewSentence,
            InjectionPrefixClass::PlainWordStart,
            true,
            "hello ",
        ));
        assert!(!should_add_leading_injection_space(
            " Hello world",
            SentenceContext::MidSentence,
            InjectionPrefixClass::PlainWordStart,
            true,
            "hello.",
        ));
    }

    #[test]
    fn sentence_boundary_adds_space_before_opening_punctuation() {
        assert!(should_add_leading_injection_space(
            "\"Hello\"",
            SentenceContext::NewSentence,
            InjectionPrefixClass::SoftPunctuationPrefix,
            true,
            "hello.",
        ));
        assert!(should_add_leading_injection_space(
            "(Hello)",
            SentenceContext::NewSentence,
            InjectionPrefixClass::SoftPunctuationPrefix,
            true,
            "hello.",
        ));
        assert!(!should_add_leading_injection_space(
            ", hello",
            SentenceContext::NewSentence,
            InjectionPrefixClass::SoftPunctuationPrefix,
            true,
            "hello.",
        ));
        assert!(!should_add_leading_injection_space(
            "\"Hello\"",
            SentenceContext::NewSentence,
            InjectionPrefixClass::SoftPunctuationPrefix,
            false,
            "hello.",
        ));
        assert!(!should_add_leading_injection_space(
            "\"Hello\"",
            SentenceContext::NewSentence,
            InjectionPrefixClass::SoftPunctuationPrefix,
            true,
            "hello ",
        ));
    }

    #[test]
    fn comma_prefix_keeps_model_casing_and_skips_leading_space() {
        let prefix = classify_leading_prefix(", I think");
        assert_eq!(prefix, InjectionPrefixClass::SoftPunctuationPrefix);
        assert_eq!(
            format_injection_text(", I think", SentenceContext::NewSentence, prefix),
            ", I think"
        );
        assert!(!should_add_injection_space(
            SentenceContext::MidSentence,
            prefix
        ));
    }

    #[test]
    fn dash_and_ellipsis_prefixes_preserve_model_casing() {
        let dash_prefix = classify_leading_prefix("\u{2014}Hello");
        assert_eq!(dash_prefix, InjectionPrefixClass::SoftPunctuationPrefix);
        assert_eq!(
            format_injection_text("\u{2014}Hello", SentenceContext::MidSentence, dash_prefix),
            "\u{2014}Hello"
        );

        let ellipsis_prefix = classify_leading_prefix("\u{2026}Hello");
        assert_eq!(ellipsis_prefix, InjectionPrefixClass::SoftPunctuationPrefix);
        assert_eq!(
            format_injection_text(
                "\u{2026}Hello",
                SentenceContext::MidSentence,
                ellipsis_prefix
            ),
            "\u{2026}Hello"
        );
    }

    #[test]
    fn hard_terminators_still_capitalize_after_they_start_the_chunk() {
        let prefix = classify_leading_prefix("!hello");
        assert_eq!(prefix, InjectionPrefixClass::HardSentenceTerminator);
        assert_eq!(
            format_injection_text("!hello", SentenceContext::MidSentence, prefix),
            "!Hello"
        );
        assert!(should_capitalize_injection(
            SentenceContext::MidSentence,
            prefix
        ));
        assert!(!should_add_injection_space(
            SentenceContext::MidSentence,
            prefix
        ));
    }
}
