//! On-device English dictation rules. This is a conservative token editor,
//! not a grammar model. Quoted text, code, markers and exact vocabulary terms
//! are barriers: neither edits nor rollback may cross them.
//! Independently implemented from the behavior described in the BetterWispr
//! research brief; no Swift source is incorporated.
//! Some regression examples are adapted from BetterWispr's Apache-2.0 test
//! cases. The supplied license is retained in licenses/BetterWispr.txt.

use unicode_segmentation::UnicodeSegmentation;

// Used only by an already-required AI cleanup call, after deterministic edits.
// Keep this compact: the shared prompt-budget estimate stays below 200 tokens.
pub(super) const MODEL_COMMAND_RECOVERY_INSTRUCTION: &str = "Before cleanup, recover only clear missed voice commands, including joined STT words. Convert comma, semicolon, full stop, question/exclamation mark; new/next line or paragraph; at sign/at the rate plus one username (at signbot becomes @bot). Period, colon, dash/hyphen require add/insert/put. Scratch/strike that removes only the latest fragment since punctuation or a line break. Remove command words, then clean normally. Preserve already-processed punctuation, line breaks and mentions; do not repeat rollback. Keep ambiguous or literal uses unchanged; never reinterpret quotes, code, vocabulary, snippets or clipboard text.";

pub(super) const MODEL_COMMAND_PRESERVATION_INSTRUCTION: &str = "Spoken commands have already been processed. Preserve the resulting punctuation and line breaks. Do not interpret any remaining words as voice commands.";

#[derive(Clone, Debug)]
struct Word {
    text: String,
    key: String,
    after: String,
    may_destutter: bool,
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '\'' | '’' | '-')
}

fn tokenize(text: &str) -> (String, Vec<Word>) {
    let mut leading = String::new();
    let mut words: Vec<Word> = Vec::new();
    let mut in_word = false;
    // Graphemes keep combining marks attached to their Unicode base letter.
    for g in text.graphemes(true) {
        if g.chars().next().is_some_and(word_char) {
            if !in_word {
                words.push(Word {
                    text: String::new(),
                    key: String::new(),
                    after: String::new(),
                    may_destutter: false,
                });
            }
            let word = words.last_mut().unwrap();
            word.text.push_str(g);
            word.key.push_str(&g.to_lowercase());
            in_word = true;
        } else {
            in_word = false;
            if let Some(word) = words.last_mut() {
                word.after.push_str(g);
            } else {
                leading.push_str(g);
            }
        }
    }
    // Preserve original casing even if removing a filler later capitalizes a
    // lowercase word. Cased names/acronyms are ambiguous; English "I" is the
    // explicit pronoun exception for a common stutter.
    for word in &mut words {
        word.may_destutter = word.text == word.key || word.text == "I";
    }
    (leading, words)
}

fn terminal(s: &str) -> bool {
    s.contains(['.', '!', '?', '…', '\n', '\r'])
}
// A period embedded between tokens belongs to a filename, domain or decimal.
// Spoken punctuation has its own explicit state; ordinary text needs a real
// separating boundary. Unambiguous terminators retain no-space behavior.
fn sentence_boundary(s: &str, at_end: bool) -> bool {
    sentence_boundary_in(s, at_end, false)
}
fn identifier_chunk(s: &str) -> bool {
    s.contains(['/', '=', '_']) || s.char_indices().any(|(i, c)| {
        (c == '.' && s[..i].chars().next_back().is_some_and(char::is_alphanumeric)
            && s[i + 1..].chars().next().is_some_and(char::is_alphanumeric))
            || (c == '@' && i > 0)
            || (c == ':' && i > 0
                && s[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == ']')
                && s[i + 1..].chars().next().is_some_and(|c| c.is_ascii_digit()))
    })
}
fn identifier_word(words: &[Word], i: usize) -> bool {
    let mut start = i;
    let mut end = i;
    while start > 0 && !words[start - 1].after.chars().any(char::is_whitespace) { start -= 1; }
    while end + 1 < words.len() && !words[end].after.chars().any(char::is_whitespace) { end += 1; }
    let chunk: String = words[start..=end].iter().map(|w| format!("{}{}", w.text, w.after)).collect();
    identifier_chunk(chunk.trim_end())
}
fn sentence_boundary_in(s: &str, at_end: bool, identifier: bool) -> bool {
    s.char_indices().any(|(i, c)| {
        matches!(c, '…' | '\n' | '\r')
            || (matches!(c, '!' | '?') && (!identifier || at_end || s[i + 1..].chars().any(char::is_whitespace)))
            || (c == '.' && (s[i + 1..].chars().any(char::is_whitespace) || at_end))
    })
}
fn inline_space(s: &str) -> bool {
    s.chars().all(|c| matches!(c, ' ' | '\t'))
}
// A tokenizer gap is not necessarily a spoken separator: dots, slashes and
// @ can join tokens into identifiers. Ordinary sentence punctuation may wrap
// a spoken token only at an edge or together with actual whitespace.
fn spoken_separator(gap: &str, at_edge: bool) -> bool {
    gap.chars().all(|c| c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '?' | '!' | '…' | '(' | ')' | '[' | ']' | '{' | '}'))
        && (at_edge || gap.chars().any(char::is_whitespace))
}
fn spoken_start(leading: &str, words: &[Word], i: usize) -> bool {
    spoken_separator(if i == 0 { leading } else { &words[i - 1].after }, i == 0)
}
fn spoken_end(words: &[Word], last: usize) -> bool {
    spoken_separator(&words[last].after, last + 1 == words.len())
}
fn capitalize(s: &mut String) {
    if *s == s.to_lowercase() {
        if let Some(c) = s.chars().next() {
            *s = c.to_uppercase().collect::<String>() + &s[c.len_utf8()..];
        }
    }
}
fn render(leading: &str, words: &[Word]) -> String {
    let mut out = leading.to_owned();
    for w in words {
        out.push_str(&w.text);
        out.push_str(&w.after);
    }
    out
}

fn number(w: &Word) -> bool {
    w.key.chars().any(char::is_numeric)
        || matches!(
            w.key.as_str(),
            "zero"
                | "oh"
                | "one"
                | "two"
                | "three"
                | "four"
                | "five"
                | "six"
                | "seven"
                | "eight"
                | "nine"
                | "ten"
        )
}

fn remove_empty_delimiters(before: &mut String, after: &mut String) -> bool {
    let mut removed = false;
    loop {
        let left = before.trim_end_matches([' ', '\t']);
        let right = after.trim_start_matches([' ', '\t']);
        let pair = left.chars().next_back().zip(right.chars().next());
        if !matches!(pair, Some(('(', ')') | ('[', ']') | ('{', '}'))) {
            return removed;
        }
        let left_end = left.len() - 1;
        let remaining = right[1..].to_owned();
        before.truncate(left_end);
        *after = remaining;
        removed = true;
    }
}

fn drop_words(leading: &mut String, words: &mut Vec<Word>, i: usize, n: usize, sentence_initial: bool) {
    let mut after = words[i + n - 1].after.clone();
    let has_following_word = i + n < words.len();
    let ends_sentence = terminal(&after);
    let capitalize_next = ends_sentence || if i == 0 {
        sentence_initial || terminal(leading)
    } else {
        terminal(&words[i - 1].after)
    };
    let before = if i == 0 {
        &mut *leading
    } else {
        &mut words[i - 1].after
    };
    let removed_pair = remove_empty_delimiters(before, &mut after);
    if removed_pair {
        // The pair enclosed only the removed pause. Its outside punctuation
        // belongs to the retained speech, including nested empty pairs.
        *before = before.trim_end_matches([' ', '\t']).to_owned()
            + after.trim_start_matches([' ', '\t']);
        if inline_space(&after) && i > 0 && has_following_word {
            before.push(' ');
        }
    } else if i == 0 {
        // Keep delimiters that enclose retained words; remove an empty pair
        // only when the discarded pause was its entire contents.
        if after.contains([')', ']', '}']) || (ends_sentence && leading.contains(['(', '[', '{'])) {
            leading.push_str(&after);
        } else {
            leading.push_str(
                &after
                    .chars()
                    .filter(|c| matches!(c, '\n' | '\r'))
                    .collect::<String>(),
            );
        }
    } else {
        let before = &mut words[i - 1].after;
        if after.contains([')', ']', '}']) {
            *before = before.trim_end_matches([' ', '\t']).to_owned()
                + after.trim_start_matches([' ', '\t']);
        } else if ends_sentence && before.contains(['(', '[', '{']) {
            before.push_str(&after);
        } else if ends_sentence && !terminal(before) {
            *before = after;
        } else if n == 2 && before.contains(',') && after.contains(',') {
            *before = " ".into();
        }
    }
    words.drain(i..i + n);
    if capitalize_next && i < words.len() {
        capitalize(&mut words[i].text);
    }
}

fn basic(text: &str, sentence_initial: bool) -> String {
    let (mut leading, mut words) = tokenize(text);
    let mut i = 0;
    while i < words.len() {
        let filler = matches!(
            words[i].key.as_str(),
            "uh" | "uhh" | "uhm" | "um" | "umm" | "er" | "erm" | "hm" | "hmm" | "mm" | "mmm"
        ) && spoken_start(&leading, &words, i) && spoken_end(&words, i)
            && !words[i].text.chars().all(|c| c.is_uppercase())
            && !(words[i].key == "mm" && i > 0 && number(&words[i - 1]));
        let you_know = words[i].key == "you"
            && i + 1 < words.len()
            && spoken_start(&leading, &words, i) && spoken_end(&words, i + 1)
            && words[i + 1].key == "know"
            && inline_space(&words[i].after)
            && (i == 0 || words[i - 1].after.contains(',') || terminal(&words[i - 1].after))
            && !words[i + 1].after.contains('?')
            && (i + 2 == words.len()
                || words[i + 1].after.contains(',')
                || terminal(&words[i + 1].after));
        if filler || you_know {
            drop_words(&mut leading, &mut words, i, if you_know { 2 } else { 1 }, sentence_initial);
        } else {
            i += 1;
        }
    }
    // A repair requires an explicit correction cue and a repeated nearby anchor.
    i = 1;
    while i < words.len() {
        if !words[i - 1].after.contains(',') || terminal(&words[i - 1].after) {
            i += 1;
            continue;
        }
        let mut end = i;
        let mut cues = 0;
        let mut explicit = false;
        while end < words.len() {
            let count = if words[end].key == "i"
                && end + 1 < words.len()
                && words[end + 1].key == "mean"
                && inline_space(&words[end].after)
            {
                explicit = true;
                2
            } else if matches!(
                words[end].key.as_str(),
                "sorry" | "no" | "wait" | "oops" | "actually"
            ) {
                explicit |= words[end].key == "no";
                1
            } else {
                break;
            };
            if terminal(&words[end + count - 1].after) {
                break;
            }
            end += count;
            cues += 1;
        }
        // A predicate anywhere in this replacement clause means the repeated
        // anchor can introduce a new statement, rather than a correction.
        let clause_end = words[end..]
            .iter()
            .position(|word| terminal(&word.after) || word.after.contains(';'))
            .map_or(words.len(), |offset| end + offset + 1);
        if explicit
            && end < words.len()
            && cues > 0
            && (cues > 1 || words[end - 1].after.contains(','))
            // Only phrase introducers can anchor a replacement. A repeated
            // content word can instead begin a new clause ("tea is gone").
            && matches!(words[end].key.as_str(), "a" | "an" | "the" | "to" | "from" | "at" | "in" | "on" | "with" | "for")
            && !words[end..clause_end].iter().any(|word| matches!(word.key.as_str(),
                "is" | "are" | "was" | "were" | "has" | "have" | "had" | "will" | "would" | "can" | "could" | "should" | "must" | "does" | "do" | "did"))
        {
            let anchor = (i.saturating_sub(4)..i).rev().find(|&a| {
                words[a].key == words[end].key
                    && !number(&words[a])
                    && words[a..i].iter().all(|w| !terminal(&w.after))
            });
            if let Some(a) = anchor {
                words[a].after = words[end].after.clone();
                words.drain(a + 1..=end);
                i = a + 1;
                continue;
            }
        }
        i += 1;
    }
    i = 0;
    while i < words.len() {
        let mut removed = false;
        let mut sentence_case = words[i].key.clone();
        capitalize(&mut sentence_case);
        let at_sentence_start = if i == 0 {
            sentence_initial || sentence_boundary(&leading, false)
        } else {
            sentence_boundary(&words[i - 1].after, false)
        };
        for n in (1..=3).rev() {
            if i + 2 * n > words.len() {
                continue;
            }
            if words[i..i + n].iter().any(number)
                || words[i..i + 2 * n].iter().enumerate().any(|(j, word)| {
                    // Ordinary sentence capitalization can differ only on the
                    // first word; a cased repeated copy remains protected.
                    !word.may_destutter && !(j == 0 && at_sentence_start
                        && word.text == sentence_case
                        && words[i + n].text == words[i + n].key)
                })
            {
                continue;
            }
            if n == 1
                && matches!(
                    words[i].key.as_str(),
                    "that" | "had" | "is" | "very" | "really" | "long" | "bye" | "no" | "ha"
                )
            {
                continue;
            }
            if (0..n).all(|j| words[i + j].key == words[i + n + j].key)
                && words[i..i + 2 * n - 1]
                    .iter()
                    .all(|w| inline_space(&w.after))
            {
                words[i + n - 1].after = words[i + 2 * n - 1].after.clone();
                words.drain(i + n..i + 2 * n);
                removed = true;
                break;
            }
        }
        if !removed {
            i += 1;
        }
    }
    render(&leading, &words)
}

fn noun(s: &str) -> bool {
    matches!(
        s,
        "a" | "an" | "the" | "this" | "that" | "oxford" | "serial"
    )
}
fn phrase(words: &[Word], i: usize, keys: &[&str]) -> bool {
    i + keys.len() <= words.len()
        && keys.iter().enumerate().all(|(j, k)| words[i + j].key == *k)
        && words[i..i + keys.len() - 1]
            .iter()
            .all(|w| !w.after.is_empty() && inline_space(&w.after))
        && spoken_end(words, i + keys.len() - 1)
}

fn punctuation_phrase(words: &[Word], i: usize, keys: &[&str]) -> bool {
    if phrase(words, i, keys) { return true; }
    // An intrinsically multiword spoken punctuation phrase remains explicit
    // when ASR glues its redundant punctuation to the next word. Single-word
    // commands never get this exception: comma.com is an identifier.
    keys.len() > 1 && i + keys.len() < words.len()
        && keys.iter().enumerate().all(|(j, k)| words[i + j].key == *k)
        && words[i..i + keys.len() - 1].iter().all(|w| !w.after.is_empty() && inline_space(&w.after))
        && !words[i + keys.len() - 1].after.is_empty()
        && words[i + keys.len() - 1].after.chars().all(|c| matches!(c, '.' | '?' | '!' | ',' | ';' | ':'))
}

fn rollback(out: &mut String, needs_separator: bool) {
    let trimmed =
        out.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '.' | '?' | '!' | '…' | ',' | ';' | ':'));
    let keep = trimmed
        .char_indices()
        .rev()
        .find(|(i, c)| {
            let start = trimmed[..*i].rfind(char::is_whitespace).map_or(0, |j| j + trimmed[j..].chars().next().unwrap().len_utf8());
            let end = trimmed[*i..].find(char::is_whitespace).map_or(trimmed.len(), |j| i + j);
            matches!(c, '…' | '\n' | '\r')
                || (matches!(c, '!' | '?')
                    && (!identifier_chunk(&trimmed[start..end])
                        || trimmed[i + c.len_utf8()..].chars().next().is_none_or(char::is_whitespace)))
                || (matches!(c, '.' | ';' | ':')
                    && trimmed[i + c.len_utf8()..].chars().next().is_none_or(char::is_whitespace))
        })
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or_else(|| out.len() - out.trim_start_matches([' ', '\t']).len());
    out.truncate(keep);
    if needs_separator && !out.is_empty() && !out.ends_with(char::is_whitespace) {
        out.push(' ');
    }
}

fn commands(text: &str) -> String {
    let (leading, words) = tokenize(text);
    let mut out = leading.clone();
    let mut i = 0;
    let mut cap_next = false;
    while i < words.len() {
        let standalone = spoken_start(&leading, &words, i);
        let mut start = i;
        let requested = matches!(words[i].key.as_str(), "add" | "insert" | "put");
        if requested {
            start += 1;
            if start < words.len() && noun(&words[start].key) {
                start += 1;
            }
        }
        // "Keep this semicolon ... scratch that" dictates a retained clause.
        // Keep noun-marker protection for literal punctuation discussion.
        let retained_clause = words[i].key == "semicolon"
            && i >= 2
            && matches!(words[i - 1].key.as_str(), "this" | "that")
            && words[i - 2].key == "keep";
        let literal = !standalone || (!requested && !retained_clause && i > 0 && noun(&words[i - 1].key));
        let punct = [
            (&["comma"][..], ",", false),
            (&["semicolon"][..], ";", false),
            (&["full", "stop"][..], ".", false),
            (&["question", "mark"][..], "?", false),
            (&["exclamation", "mark"][..], "!", false),
            (&["exclamation", "point"][..], "!", false),
            (&["period"][..], ".", true),
            (&["colon"][..], ":", true),
            (&["dash"][..], "-", true),
            (&["hyphen"][..], "-", true),
        ]
        .into_iter()
        .find(|(keys, _, needs_request)| {
            !literal
                && punctuation_phrase(&words, start, keys)
                && (!needs_request
                    || requested
                    || (keys[0] == "period" && start + 1 == words.len()))
                && words[i..start].iter().all(|w| inline_space(&w.after))
        });
        if let Some((keys, symbol, _)) = punct {
            let last = start + keys.len() - 1;
            while out.ends_with([' ', '\t']) {
                out.pop();
            }
            if !out.ends_with(symbol) {
                out.push_str(symbol);
            }
            // ASR punctuation immediately around a spoken command is redundant.
            let after = words[last]
                .after
                .trim_start_matches([',', '.', '?', '!', ';', ':']);
            if i + (last - i) + 1 < words.len() && !after.starts_with(char::is_whitespace) {
                out.push(' ');
            }
            out.push_str(after);
            cap_next = matches!(symbol, "." | "?" | "!");
            i = last + 1;
            continue;
        }
        let line = !literal
            && (phrase(&words, i, &["new", "line"]) || phrase(&words, i, &["next", "line"]));
        let paragraph = !literal
            && (phrase(&words, i, &["new", "paragraph"])
                || phrase(&words, i, &["next", "paragraph"]));
        if line || paragraph {
            while out.ends_with([' ', '\t']) {
                out.pop();
            }
            out.push_str(if paragraph { "\n\n" } else { "\n" });
            cap_next = true;
            i += 2;
            continue;
        }
        let scratch = standalone
            && (phrase(&words, i, &["scratch", "that"]) || phrase(&words, i, &["strike", "that"]));
        let removal = standalone && i + 1 < words.len()
            && spoken_end(&words, i + 1)
            && words[i + 1].key == "that"
            && inline_space(&words[i].after)
            && matches!(
                words[i].key.as_str(),
                "remove" | "delete" | "undo" | "cancel"
            )
            && (i == 0 || words[i - 1].after.contains(',') || terminal(&words[i - 1].after))
            && (i + 2 == words.len()
                || words[i + 1].after.contains(',')
                || terminal(&words[i + 1].after));
        if scratch || removal {
            // Apology words belong to the command, not the retained sentence.
            if i > 0 && matches!(words[i - 1].key.as_str(), "sorry" | "oops" | "wait") {
                let suffix = format!("{}{}", words[i - 1].text, words[i - 1].after);
                if out.ends_with(&suffix) {
                    out.truncate(out.len() - suffix.len());
                }
            }
            rollback(&mut out, i + 2 < words.len() || words[i + 1].after.ends_with(char::is_whitespace));
            cap_next = true;
            i += 2;
            continue;
        }
        let mention = if phrase(&words, i, &["at", "the", "rate"]) {
            3
        } else if phrase(&words, i, &["at", "sign"]) {
            2
        } else {
            0
        };
        if standalone && mention > 0
            && i + mention < words.len()
            && words[i + mention].key != "of"
            && inline_space(&words[i + mention - 1].after)
        {
            out.push('@');
            out.push_str(&words[i + mention].text);
            out.push_str(&words[i + mention].after);
            // The username consumes the pending sentence position just like
            // an ordinary word; its trailing punctuation starts the next one.
            cap_next = sentence_boundary_in(&words[i + mention].after, i + mention + 1 == words.len(), identifier_word(&words, i + mention));
            i += mention + 1;
            continue;
        }
        let mut word = words[i].text.clone();
        if words[i].key != "of" && out.trim_end_matches([' ', '\t']).ends_with('@') {
            while out.ends_with([' ', '\t']) {
                out.pop();
            }
        }
        if cap_next {
            capitalize(&mut word);
        }
        out.push_str(&word);
        out.push_str(&words[i].after);
        cap_next = sentence_boundary_in(&words[i].after, i + 1 == words.len(), identifier_word(&words, i));
        i += 1;
    }
    out
}

pub(super) fn explicit_english(language: &str) -> bool {
    language
        .split(['-', '_'])
        .next()
        .is_some_and(|s| s.eq_ignore_ascii_case("en"))
}

fn matching_term_len(text: &str, term: &str) -> Option<usize> {
    if term.is_empty() {
        return None;
    }
    let mut chars = text.char_indices();
    let mut end = 0;
    for wanted in term.chars() {
        let (index, actual) = chars.next()?;
        if !wanted.to_lowercase().eq(actual.to_lowercase()) {
            return None;
        }
        end = index + actual.len_utf8();
    }
    text[end..]
        .chars()
        .next()
        .is_none_or(|c| !word_char(c))
        .then_some(end)
}

fn protected_span_len(text: &str, i: usize, terms: &[&str]) -> Option<usize> {
    let rest = &text[i..];
    let c = rest.chars().next()?;
    let quoted = match c {
        // An attached inch mark is not the start of quoted speech. Genuine
        // opening quotes still protect unmatched spans through end of input.
        '"' if i == 0 || !text[..i].graphemes(true).next_back()
            .and_then(|g| g.chars().next()).is_some_and(word_char) => Some('"'),
        '“' => Some('”'),
        '`' => Some('`'),
        '‘' => Some('’'),
        '\'' if i == 0 || !text[..i].chars().next_back().is_some_and(word_char) => Some('\''),
        _ => None,
    };
    if rest.starts_with("[[VERENU_") {
        Some(rest.find("]]").map(|n| n + 2).unwrap_or(rest.len()))
    } else if let Some(close) = quoted {
        Some(rest[c.len_utf8()..].char_indices().find_map(|(offset, candidate)| {
            if candidate != close { return None; }
            let index = c.len_utf8() + offset;
            let is_word = |ch| word_char(ch) && !matches!(ch, '\'' | '’');
            // A contraction apostrophe is part of the quoted word, not its
            // closing delimiter. Inspect the preceding grapheme's base so
            // decomposed Unicode letters have the same boundary semantics.
            let intra_word = matches!(close, '\'' | '’')
                && rest[..index].graphemes(true).next_back()
                    .and_then(|g| g.chars().next()).is_some_and(is_word)
                && rest[index + close.len_utf8()..].chars().next().is_some_and(is_word);
            // An attached numeric inch mark inside a quote can precede its
            // actual closing quote. Prefer that larger literal span only when
            // the next quote looks like a close, not a separate quote's opener.
            // A final numeric quote still closes normally.
            let measurement_mark = close == '"'
                && rest[..index].graphemes(true).next_back()
                    .and_then(|g| g.chars().next()).is_some_and(char::is_numeric)
                && rest[index + 1..].find('"').is_some_and(|next| {
                    rest[index + 1..index + 1 + next].graphemes(true).next_back()
                        .and_then(|g| g.chars().next())
                        .is_some_and(|c| is_word(c) || matches!(c, '.' | '!' | '?' | '…' | ')' | ']' | '}'))
                });
            (!intra_word && !measurement_mark).then_some(index + close.len_utf8())
        }).unwrap_or(rest.len()))
    } else {
        terms.iter()
            .filter(|_| i == 0 || !text[..i].chars().next_back().is_some_and(word_char))
            .filter_map(|term| matching_term_len(rest, term)).max()
    }
}

/// Expanded payloads are not labeled for the model. In their presence keep
/// model command recovery off rather than asking it to guess their origin.
pub(super) fn has_protected_spans(text: &str, terms: &[&str]) -> bool {
    text.char_indices().any(|(i, _)| protected_span_len(text, i, terms).is_some())
}

#[cfg(test)]
pub(super) fn process(text: &str, cleanup: bool, voice_commands: bool, terms: &[&str]) -> String {
    process_after(text, cleanup, voice_commands, terms, "")
}

fn sentence_start(prefix: &str) -> bool {
    if prefix.trim().is_empty() {
        return true;
    }
    // Closing delimiters do not hide punctuation ending a quoted sentence.
    let end = prefix.trim_end_matches([' ', '\t', '"', '”', '\'', '’', '`', ')', ']', '}']);
    end.chars().next_back().is_some_and(|c| matches!(c, '.' | '!' | '?' | '…' | '\n' | '\r'))
}

/// Edit spoken spans independently while retaining sentence context across
/// protected payloads. Unclosed quotes/code protect the remaining transcript.
pub(super) fn process_after(text: &str, cleanup: bool, voice_commands: bool, terms: &[&str], prefix: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut spoken_start = 0;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let c = rest.chars().next().unwrap();
        let protected_len = protected_span_len(text, i, terms);
        if let Some(len) = protected_len {
            let sentence_initial = sentence_start(if out.is_empty() { prefix } else { &out });
            out.push_str(&edit(&text[spoken_start..i], cleanup, voice_commands, sentence_initial));
            out.push_str(&text[i..i + len]);
            i += len;
            spoken_start = i;
        } else {
            i += c.len_utf8();
        }
    }
    let sentence_initial = sentence_start(if out.is_empty() { prefix } else { &out });
    out.push_str(&edit(&text[spoken_start..], cleanup, voice_commands, sentence_initial));
    out
}

fn edit(text: &str, cleanup: bool, voice_commands: bool, sentence_initial: bool) -> String {
    // Execute each command before Basic can collapse repeated spoken words.
    // Generated line boundaries remain boundaries during mechanical cleanup.
    let text = if voice_commands {
        commands(text)
    } else {
        text.to_owned()
    };
    if cleanup {
        basic(&text, sentence_initial)
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measurement_marks_inside_quotes_do_not_close_literal_spans() {
        for input in [
            "say \"the board is 5\" new line please\" literally",
            "say \"the board is 5\" um please\" literally",
            "say \"the board is 5\" scratch that please\" literally",
            "say \"the board is 5.5\" new line please\" literally",
            "say \"the board is 5\" by 6\" new line please\" literally",
        ] {
            assert_eq!(process(input, true, true, &[]), input, "{input}");
        }
        assert_eq!(process("say \"size 5\" new line tomorrow", true, true, &[]), "say \"size 5\"\nTomorrow");
        assert_eq!(process("say \"size 5\" new line \"um please\"", true, true, &[]), "say \"size 5\"\n\"um please\"");
        assert_eq!(process("say \"5\" new line \"6\"", true, true, &[]), "say \"5\"\n\"6\"");
    }

    #[test]
    fn rollback_preserves_identifier_sentence_punctuation() {
        for (input, expected) in [
            ("Did you visit example.com? change this scratch that tomorrow", "Did you visit example.com? Tomorrow"),
            ("Visit https://example.com! change this scratch that tomorrow", "Visit https://example.com! Tomorrow"),
            ("Visit localhost:3000? change this scratch that tomorrow", "Visit localhost:3000? Tomorrow"),
            ("Visit example.com?query=value scratch that tomorrow", "Tomorrow"),
            ("Hello. visit example.com?query=value scratch that tomorrow", "Hello. Tomorrow"),
        ] {
            assert_eq!(process(input, false, true, &[]), expected, "{input}");
        }
    }

    #[test]
    fn empty_filler_delimiters_preserve_external_punctuation() {
        for (input, expected) in [
            ("Please (um), continue", "Please, continue"),
            ("Please [um]; continue", "Please; continue"),
            ("Please {um}: continue", "Please: continue"),
            ("Please ([um]), continue", "Please, continue"),
            ("Please (um). continue", "Please. Continue"),
            ("Please (um)! continue", "Please! Continue"),
            ("Please (um)? continue", "Please? Continue"),
            ("Please, um, continue", "Please, continue"),
        ] {
            assert_eq!(process(input, true, false, &[]), expected, "{input}");
        }
    }

    #[test]
    fn measurement_quotes_do_not_protect_following_speech() {
        assert_eq!(process("make it 5\" wide um please", true, false, &[]), "make it 5\" wide please");
        assert_eq!(process("make it 5\" wide new line please", true, true, &[]), "make it 5\" wide\nPlease");
        assert_eq!(process("make it 5\" wide new line please", false, false, &[]), "make it 5\" wide new line please");
        for input in ["say \"um new line\" literally", "say \"um new line", "\"5 inch um new line\"", "`5\" wide um new line`", "[[VERENU_CLIPBOARD_5\" um new line]]"] {
            assert_eq!(process(input, true, true, &[]), input, "{input}");
        }
    }
    #[test]
    fn url_query_delimiters_are_not_sentence_boundaries() {
        for input in ["visit example.com?query=value", "visit https://example.com/path?query=value", "open report.md!section", "visit example.com?query", "visit localhost:3000?debug", "visit localhost:3000!debug"] {
            assert_eq!(process(input, false, true, &[]), input, "{input}");
            assert_eq!(process(&format!("{input} scratch that"), false, true, &[]), "", "{input}");
            assert_eq!(process(&format!("Hello. {input} scratch that tomorrow"), false, true, &[]), "Hello. Tomorrow", "{input}");
        }
        assert_eq!(process("hello!tomorrow", false, true, &[]), "hello!Tomorrow");
        assert_eq!(process("Okay? tomorrow", false, true, &[]), "Okay? Tomorrow");
    }
    #[test]
    fn rollback_uses_real_clause_boundaries() {
        for input in ["open report.md scratch that", "visit example.com scratch that", "version 1.25 scratch that", "visit https://example.com scratch that", "open /tmp/report.md scratch that", "visit https://example.com/report.md scratch that", "Hello. scratch that", "Hello… scratch that", "Keep this put a colon scratch that"] {
            assert_eq!(process(input, true, true, &[]), "", "{input}");
        }
        for (input, expected) in [
            ("Hi team. open report.md scratch that tomorrow", "Hi team. Tomorrow"),
            ("Hi team. version 1.25 scratch that tomorrow", "Hi team. Tomorrow"),
            ("Keep this: visit https://example.com scratch that tomorrow", "Keep this: Tomorrow"),
            ("Keep it; visit example.com scratch that tomorrow", "Keep it; Tomorrow"),
            ("Hello… world scratch that", "Hello…"),
            ("Hello… world scratch that tomorrow", "Hello… Tomorrow"),
            ("Hello!world scratch that tomorrow", "Hello! Tomorrow"),
        ] {
            assert_eq!(process(input, true, true, &[]), expected, "{input}");
        }
        for input in ["open report.md scratch that", "visit https://example.com scratch that"] {
            assert_eq!(process(input, true, false, &[]), input);
        }
        assert_eq!(process("\"open report.md scratch that\"", true, true, &[]), "\"open report.md scratch that\"");
        // The protected segment's following separator is preserved verbatim.
        assert_eq!(process("[[VERENU_CLIPBOARD_report.md]] scratch that", true, true, &[]), "[[VERENU_CLIPBOARD_report.md]] ");
    }
    #[test]
    fn single_quoted_contractions_protect_fillers() {
        for input in ["say 'don't um pause' literally", "say ‘don’t um pause’ literally", "say 'don't um pause", "say ‘don’t um pause", "say 'cafe\u{301}'s um pause' literally"] {
            assert_eq!(process(input, true, true, &[]), input, "{input}");
            assert_eq!(process(input, true, false, &[]), input, "{input}");
        }
    }
    #[test]
    fn single_quoted_contractions_protect_commands() {
        for input in ["say 'don't new line please' literally", "say ‘don’t new line please’ literally", "say 'don't scratch that please' literally", "say ‘l’été new line please’ literally", "say 'don't new line please", "say ‘don’t new line please"] {
            assert_eq!(process(input, false, true, &[]), input, "{input}");
            assert_eq!(process(input, true, true, &[]), input, "{input}");
        }
        for (input, expected) in [("say 'don't new line' comma tomorrow", "say 'don't new line', tomorrow"), ("say ‘don’t new line’ comma tomorrow", "say ‘don’t new line’, tomorrow")] {
            assert_eq!(process(input, true, true, &[]), expected, "{input}");
        }
        for input in ["say \"don't um new line\" literally", "say `don't um new line` literally", "[[VERENU_CLIPBOARD_don't um new line]]"] {
            assert_eq!(process(input, true, true, &[]), input);
        }
        assert_eq!(process("don't um new line", true, true, &["don't um new line"]), "don't um new line");
    }
    #[test]
    fn fillers_require_standalone_spoken_tokens() {
        for input in ["contact um@example.com", "visit um.edu", "open /um/report.md", "open um.txt", "value um_value", "use um-value", "address a@um.edu", "version 1.um", "use (um.edu)"] {
            assert_eq!(process(input, true, false, &[]), input, "{input}");
            assert_eq!(process(input, true, true, &[]), input, "{input}");
        }
        for (input, expected) in [("um, send it", "Send it"), ("send, um, it", "send, it"), ("send (um) it", "send it"), ("um. send it", "Send it")] {
            assert_eq!(process(input, true, false, &[]), expected, "{input}");
        }
    }
    #[test]
    fn command_phrases_require_spoken_identifier_boundaries() {
        for input in ["visit comma.com", "open semicolon.txt", "mail comma@example.com", "open /comma/file", "open /tmp/comma", "value comma_value", "use comma-value", "version 1.comma", "open new line.txt", "open scratch that.txt", "visit example.comma", "open full.stop", "put a comma.com"] {
            assert_eq!(process(input, false, true, &[]), input, "{input}");
            assert_eq!(process(input, true, true, &[]), input, "{input}");
        }
        for (input, expected) in [("hello comma, world", "hello, world"), ("hello semicolon. tomorrow", "hello; tomorrow"), ("hello full stop.tomorrow", "hello. Tomorrow"), ("hello question mark?tomorrow", "hello? Tomorrow"), ("hello comma", "hello,"), ("hello comma. world", "hello, world")] {
            assert_eq!(process(input, false, true, &[]), expected, "{input}");
        }
        for input in ["\"um comma.com\"", "`um comma.com`", "[[VERENU_CLIPBOARD_um comma.com]]"] {
            assert_eq!(process(input, true, true, &[]), input);
        }
        assert_eq!(process("um comma", true, true, &["um comma"]), "um comma");
        assert_eq!(process("um comma", false, false, &[]), "um comma");
    }
    #[test]
    fn commands_preserve_embedded_periods_and_real_sentence_boundaries() {
        for input in ["open report.md", "visit example.com", "open /tmp/report.md", "version 1.25", "visit https://example.com/report.md", "load config.json.value"] {
            assert_eq!(process(input, false, true, &[]), input, "{input}");
            assert_eq!(process(input, true, true, &[]), input, "{input}");
        }
        for (input, expected) in [
            ("open report.md. tomorrow", "open report.md. Tomorrow"),
            ("hello full stop.tomorrow", "hello. Tomorrow"),
            ("hello full stop tomorrow", "hello. Tomorrow"),
            ("hello!tomorrow", "hello!Tomorrow"),
            ("ping at sign maria. tomorrow", "ping @maria. Tomorrow"),
        ] {
            assert_eq!(process(input, false, true, &[]), expected, "{input}");
        }
        for input in ["\"open report.md\"", "`visit example.com`", "[[VERENU_CLIPBOARD_report.md]]"] {
            assert_eq!(process(input, true, true, &[]), input);
        }
    }
    #[test]
    fn basic_sentence_initial_stutters_work_after_real_terminators() {
        for (input, expected) in [
            ("Okay. Go go now", "Okay. Go now"),
            ("Okay! Send send it", "Okay! Send it"),
            ("Okay? Go go now", "Okay? Go now"),
            ("Okay\nGo go now", "Okay\nGo now"),
        ] {
            assert_eq!(process(input, true, false, &[]), expected, "{input}");
            assert_eq!(process(input, true, true, &[]), expected, "{input}");
            assert_eq!(process(input, false, false, &[]), input, "{input}");
        }
        for input in ["Okay. Duran Duran", "Okay. Bora Bora", "Okay. NASA NASA", "Okay. NASA nasa", "Okay. iPhone iPhone", "report.Go go", "\"Okay. Go go now\"", "`Okay. Go go now`"] {
            assert_eq!(process(input, true, false, &[]), input, "{input}");
        }
    }
    #[test]
    fn model_command_recovery_is_compact_and_preserves_payload_boundaries() {
        let prompt = crate::api::prompts::get_cleanup_prompt_with_extras(
            "groq", "llama-3.3-70b-versatile", "casual", "light",
            MODEL_COMMAND_RECOVERY_INSTRUCTION, None, "ping at signbot", None,
        );
        let baseline = crate::api::prompts::get_cleanup_prompt_with_extras(
            "groq", "llama-3.3-70b-versatile", "casual", "light", "", None,
            "ping at signbot", None,
        );
        assert!(crate::api::prompts::prompt_token_estimate(&prompt)
            - crate::api::prompts::prompt_token_estimate(&baseline) < 200);
        assert!(prompt.contains("at signbot becomes @bot"));
        assert!(!MODEL_COMMAND_RECOVERY_INSTRUCTION.contains("Do not interpret any remaining"));
        for text in ["\"at signbot\"", "‘at signbot’", "`at signbot`", "\"unclosed at signbot", "[[VERENU_CLIPBOARD_at signbot]]", "Use CommandName", "use commandname"] {
            assert!(has_protected_spans(text, &["CommandName"]), "{text}");
        }
        for text in ["ping at signbot", "don't send it", "the commandname_suffix", "aCommandName"] {
            assert!(!has_protected_spans(text, &["CommandName"]), "{text}");
        }
        // The deterministic parser remains conservative; the model receives
        // residual variants, not a new global fuzzy command interpretation.
        assert_eq!(process("ping at signbot", false, true, &[]), "ping at signbot");
    }
    #[test]
    fn mention_trailing_sentence_terminator_capitalizes_following_word() {
        assert_eq!(process("ping at sign maria. tomorrow", true, true, &[]), "ping @maria. Tomorrow");
    }
    #[test]
    fn mention_refreshes_sentence_state_after_consuming_username() {
        for (input, expected) in [
            ("Hello full stop at sign maria tomorrow", "Hello. @maria tomorrow"),
            ("ping at sign maria. tomorrow", "ping @maria. Tomorrow"),
            ("Hello full stop at the rate maria tomorrow", "Hello. @maria tomorrow"),
            ("ping at the rate maria! tomorrow", "ping @maria! Tomorrow"),
        ] {
            assert_eq!(process(input, true, true, &[]), expected, "{input}");
            assert_eq!(process(input, false, true, &[]), expected, "{input}");
            assert_eq!(process(input, true, false, &[]), input, "{input}");
        }
        for input in ["\"ping at sign maria. tomorrow\"", "`ping at sign maria. tomorrow`", "[[VERENU_CLIPBOARD_ping at sign maria. tomorrow]]"] {
            assert_eq!(process(input, true, true, &[]), input);
        }
        assert_eq!(process("ping at sign maria. tomorrow", true, true, &["at sign maria. tomorrow"]), "ping at sign maria. tomorrow");
    }
    #[test]
    fn rollback_preserves_colon_clause_and_payload_boundaries() {
        for (input, expected) in [
            ("Keep this put a colon change this scratch that tomorrow", "Keep this: Tomorrow"),
            ("Keep this: change this scratch that tomorrow", "Keep this: Tomorrow"),
            ("Keep this put a colon scratch that tomorrow", "Tomorrow"),
            ("before [[VERENU_CLIPBOARD_7D3A_00]] Keep this put a colon change this scratch that tomorrow", "before [[VERENU_CLIPBOARD_7D3A_00]] Keep this: Tomorrow"),
            ("\"Keep this put a colon change this scratch that\"", "\"Keep this put a colon change this scratch that\""),
            ("`Keep this put a colon change this scratch that`", "`Keep this put a colon change this scratch that`"),
        ] {
            assert_eq!(process(input, true, true, &[]), expected, "{input}");
        }
        let literal = "Keep this put a colon change this scratch that tomorrow";
        assert_eq!(process(literal, true, false, &[]), literal);
    }
    #[test]
    fn basic_preserves_cased_repetition_without_losing_lowercase_stutters() {
        for input in ["Duran Duran", "Bora Bora", "NASA NASA", "NASA nasa", "US US", "New York New York", "iPhone iPhone"] {
            assert_eq!(process(input, true, false, &[]), input);
            assert_eq!(process(input, true, true, &[]), input);
        }
        for (input, expected) in [
            ("I I I think so", "I think so"),
            ("send send it", "send it"),
            ("Send send it", "Send it"),
            ("um send send it", "Send it"),
            ("Then we can we can deploy", "Then we can deploy"),
        ] {
            assert_eq!(process(input, true, false, &[]), expected, "{input}");
        }
    }
    #[test]
    fn basic_preserves_each_repeated_voice_command() {
        for (input, expected) in [
            ("First. Second. scratch that scratch that", ""),
            ("Hello new line new line tomorrow", "Hello\n\nTomorrow"),
            ("Hello comma comma tomorrow", "Hello, tomorrow"),
        ] {
            assert_eq!(process(input, true, true, &[]), expected, "{input}");
        }
        assert_eq!(process("Scratch that scratch that", true, false, &[]), "Scratch that");
        assert_eq!(process("\"scratch that scratch that\"", true, true, &[]), "\"scratch that scratch that\"");
        assert_eq!(process("`new line new line`", true, true, &[]), "`new line new line`");
        assert_eq!(process("new line new line", true, true, &["new line"]), "new line new line");
    }
    #[test]
    fn basic_examples_and_literal_boundaries() {
        for (input, expected) in [
            ("um hello, um world!", "Hello, world!"),
            (
                "Then we can we can deploy the app",
                "Then we can deploy the app",
            ),
            ("I I I think so", "I think so"),
            ("It was, you know, fine.", "It was fine."),
            (
                "I want to go to Ahmedabad, sorry, no, to Delhi.",
                "I want to go to Delhi.",
            ),
            (
                "Book the window seat, I mean, the aisle seat",
                "Book the aisle seat",
            ),
            (
                "Book the window seat, I mean, the aisle seat. It is available.",
                "Book the aisle seat. It is available.",
            ),
            ("Cut it to 5 mm, mm, thanks", "Cut it to 5 mm, thanks"),
            ("um iPhone works", "iPhone works"),
            ("um", ""),
            ("yes. um tomorrow", "yes. Tomorrow"),
            ("écho écho café", "écho café"),
        ] {
            assert_eq!(process(input, true, false, &[]), expected, "{input}");
        }
        for input in [
            "I know that that works",
            "a long long time ago",
            "call 5 5 5 now",
            "dial one one two",
            "Hello, hello, hello, is this on?",
            "You know the answer.",
            "Do you know?",
            "Tuesday, I mean, Wednesday",
            "I said yes to the plan, no to the budget.",
            "Is it at 5? No, at 6.",
            "Go to Ahmedabad, sorry, no, Delhi",
            "I ordered tea, no, tea is unavailable.",
            "I ordered the tea, no, the tea is unavailable.",
            "I chose coffee, I mean, coffee tastes better.",
            "I met the doctor, no, the doctor who called me was a nurse",
            "yes\r\nyes",
            "yes\n\nyes",
            "mm-hmm uh-huh",
            "\"um um scratch that\"",
            "`um um`",
        ] {
            assert_eq!(process(input, true, false, &[]), input);
        }
    }
    #[test]
    fn commands_and_protected_spans() {
        for (input, expected) in [
            (
                "hello comma world full stop tomorrow",
                "hello, world. Tomorrow",
            ),
            ("hello new line world next paragraph", "hello\nWorld\n\n"),
            ("insert a comma here", ", here"),
            ("hello insert dash world", "hello- world"),
            (
                "Hi team. Ship it today scratch that tomorrow",
                "Hi team. Tomorrow",
            ),
            (
                "Send it Monday. Sorry, remove that. Send it Tuesday.",
                "Send it Tuesday.",
            ),
            ("ping at the rate KV about it", "ping @KV about it"),
            ("hello comma, world", "hello, world"),
            ("ping @ KV about it", "ping @KV about it"),
            (
                "before [[VERENU_CLIPBOARD_7D3A_00]] erase scratch that after",
                "before [[VERENU_CLIPBOARD_7D3A_00]] After",
            ),
        ] {
            assert_eq!(process(input, false, true, &[]), expected, "{input}");
        }
        for input in [
            "the period ended",
            "I love the Oxford comma",
            "the new paragraph is clearer",
            "Please remove that file",
            "growing at the rate of 5 percent",
            "at sign",
            "\"scratch that\" is a phrase",
            "say ‘new line’ literally",
            "don't change contractions",
        ] {
            assert_eq!(process(input, false, true, &[]), input);
        }
        assert_eq!(
            process("um Very Very um nice", true, false, &["Very Very"]),
            "Very Very nice"
        );
        assert!(explicit_english("EN-us"));
        assert!(explicit_english("en_GB"));
        assert!(!explicit_english("auto"));
        assert!(!explicit_english("fr"));
        let long = "word ".repeat(10_000);
        assert_eq!(process(&long, true, false, &[]), "word ");
    }

    #[test]
    fn clipboard_payload_restores_after_spoken_rollback() {
        let payload = "um um scratch that\r\nnew paragraph 🦀";
        let plan = super::super::clipboard_phrase::replace_phrase_with_marker(
            "um paste clipboard here discard scratch that tomorrow",
            "paste clipboard here",
            payload.into(),
        ).unwrap();
        let cleaned = process(&plan.pre_cleanup, true, true, &[]);
        assert_eq!(
            super::super::clipboard_phrase::restore(&cleaned, &plan).unwrap(),
            format!("{payload} Tomorrow"),
        );
        let normalized = process("um send send it new line tomorrow", true, true, &[]);
        assert_eq!(process(&normalized, true, true, &[]), normalized);
    }

    #[test]
    fn rollback_preserves_semicolon_clause_and_payload_boundaries() {
        for (input, expected) in [
            ("Keep this semicolon change this scratch that", "Keep this;"),
            ("Keep that semicolon change this scratch that", "Keep that;"),
            ("Keep it semicolon change this scratch that", "Keep it;"),
            ("Keep this; change this scratch that tomorrow", "Keep this; Tomorrow"),
            ("Keep this semicolon scratch that tomorrow", "Tomorrow"),
            ("before [[VERENU_CLIPBOARD_7D3A_00]] keep this semicolon change this scratch that", "before [[VERENU_CLIPBOARD_7D3A_00]] keep this;"),
            ("\"Keep this; change this scratch that\"", "\"Keep this; change this scratch that\""),
            ("Explain this semicolon", "Explain this semicolon"),
            ("Keep the semicolon", "Keep the semicolon"),
        ] {
            assert_eq!(process(input, false, true, &[]), expected, "{input}");
        }
    }

    #[test]
    fn filler_removal_preserves_uppercase_acronyms() {
        for (input, expected) in [
            ("Take me to the ER", "Take me to the ER"),
            ("I studied at UM", "I studied at UM"),
            ("UM offers courses", "UM offers courses"),
            ("Um, take me to the ER", "Take me to the ER"),
            ("I studied, um, at UM", "I studied, at UM"),
            ("Er, please continue", "Please continue"),
        ] {
            assert_eq!(process(input, true, false, &[]), expected, "{input}");
        }
    }

    #[test]
    fn filler_removal_keeps_delimiters_balanced() {
        for (input, expected) in [
            ("Please (um) continue", "Please continue"),
            ("Please [um] continue", "Please continue"),
            ("Please (um continue)", "Please (continue)"),
            ("(Please um) continue", "(Please) continue"),
            ("(um) hello", "Hello"),
            ("(um)", ""),
        ] {
            assert_eq!(process(input, true, false, &[]), expected, "{input}");
        }
    }

    #[test]
    fn protected_spans_preserve_sentence_case() {
        for (input, expected) in [
            ("I use Verenu um every day", "I use Verenu every day"),
            ("I use \"Verenu\" um every day", "I use \"Verenu\" every day"),
            ("I use `Verenu` um every day", "I use `Verenu` every day"),
            ("I use [[VERENU_CLIPBOARD_00]] um every day", "I use [[VERENU_CLIPBOARD_00]] every day"),
            ("I use Verenu. um every day", "I use Verenu. Every day"),
            ("I say \"Done.\" um every day", "I say \"Done.\" Every day"),
        ] {
            assert_eq!(process(input, true, false, &["Verenu"]), expected, "{input}");
        }
    }

    #[test]
    fn vocabulary_commands_are_protected_across_case_and_unicode_lengths() {
        for input in ["new line tomorrow", "NEW LINE tomorrow", "New Line tomorrow"] {
            assert_eq!(process(input, true, true, &["New Line"]), input);
        }
        assert_eq!(process("um ÉR ÉR works", true, true, &["ér ér"]), "ÉR ÉR works");
        assert_eq!(process("k k works", true, true, &["K K"]), "k k works");
        assert_eq!(process("new line tomorrow", false, true, &["new"]), "new line tomorrow");
        assert_eq!(process("renew line", false, true, &["New Line"]), "renew line");
        assert_eq!(process("um", true, true, &["u"]), "");
    }
}
