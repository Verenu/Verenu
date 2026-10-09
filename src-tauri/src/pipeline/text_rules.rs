//! On-device English dictation rules. This is a conservative token editor,
//! not a grammar model. Quoted text, code, markers and exact vocabulary terms
//! are barriers: neither edits nor rollback may cross them.
//! Independently implemented from the behavior described in the BetterWispr
//! research brief; no Swift source is incorporated.
//! Some regression examples are adapted from BetterWispr's Apache-2.0 test
//! cases. The supplied license is retained in licenses/BetterWispr.txt.

use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug)]
struct Word {
    text: String,
    key: String,
    after: String,
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
    (leading, words)
}

fn terminal(s: &str) -> bool {
    s.contains(['.', '!', '?', '…', '\n', '\r'])
}
fn inline_space(s: &str) -> bool {
    s.chars().all(|c| matches!(c, ' ' | '\t'))
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
    if removed_pair && i > 0 && before.is_empty() && inline_space(&after) && i + n < words.len() {
        words[i - 1].after = " ".into();
    }
    if i == 0 {
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
        ) && !(words[i].key == "mm" && i > 0 && number(&words[i - 1]));
        let you_know = words[i].key == "you"
            && i + 1 < words.len()
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
        if explicit
            && end < words.len()
            && cues > 0
            && (cues > 1 || words[end - 1].after.contains(','))
            && !matches!(
                words[end].key.split(['\'', '’']).next().unwrap_or(""),
                "i" | "we" | "you" | "he" | "she" | "it" | "they"
            )
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
        for n in (1..=3).rev() {
            if i + 2 * n > words.len() {
                continue;
            }
            if words[i..i + n].iter().any(number) {
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
            .all(|w| inline_space(&w.after))
}

fn rollback(out: &mut String) {
    let trimmed =
        out.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '.' | '?' | '!' | ','));
    let keep = trimmed
        .char_indices()
        .rev()
        .find(|(_, c)| matches!(c, '.' | '!' | '?' | '\n' | '\r'))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or_else(|| out.len() - out.trim_start_matches([' ', '\t']).len());
    out.truncate(keep);
    if !out.is_empty() && !out.ends_with(char::is_whitespace) {
        out.push(' ');
    }
}

fn commands(text: &str) -> String {
    let (leading, words) = tokenize(text);
    let mut out = leading;
    let mut i = 0;
    let mut cap_next = false;
    while i < words.len() {
        let mut start = i;
        let requested = matches!(words[i].key.as_str(), "add" | "insert" | "put");
        if requested {
            start += 1;
            if start < words.len() && noun(&words[start].key) {
                start += 1;
            }
        }
        let literal = !requested && i > 0 && noun(&words[i - 1].key);
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
                && phrase(&words, start, keys)
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
        let scratch =
            phrase(&words, i, &["scratch", "that"]) || phrase(&words, i, &["strike", "that"]);
        let removal = i + 1 < words.len()
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
            rollback(&mut out);
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
        if mention > 0
            && i + mention < words.len()
            && words[i + mention].key != "of"
            && inline_space(&words[i + mention - 1].after)
        {
            out.push('@');
            out.push_str(&words[i + mention].text);
            out.push_str(&words[i + mention].after);
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
        cap_next = terminal(&words[i].after);
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
        let quoted = match c {
            '"' => Some('"'),
            '“' => Some('”'),
            '`' => Some('`'),
            '‘' => Some('’'),
            '\'' if i == 0 || !text[..i].chars().next_back().is_some_and(word_char) => Some('\''),
            _ => None,
        };
        let protected_len = if rest.starts_with("[[VERENU_") {
            Some(rest.find("]]").map(|n| n + 2).unwrap_or(rest.len()))
        } else if let Some(close) = quoted {
            Some(
                rest[c.len_utf8()..]
                    .find(close)
                    .map(|n| c.len_utf8() + n + close.len_utf8())
                    .unwrap_or(rest.len()),
            )
        } else {
            terms
                .iter()
                .filter(|_| i == 0 || !text[..i].chars().next_back().is_some_and(word_char))
                .filter_map(|term| matching_term_len(rest, term))
                .max()
        };
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
    let text = if cleanup {
        basic(text, sentence_initial)
    } else {
        text.to_owned()
    };
    if voice_commands {
        commands(&text)
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
