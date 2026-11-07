//! Verenu vocabulary shared into a paired T3 Code environment's shared
//! dictionary (`speechProjectCustomWords`). T3 owns the list; Verenu only
//! manages the entries it wrote earlier and leaves everything else alone.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// Limits enforced by T3's `SpeechCustomWords` schema.
pub const MAX_ENTRIES: usize = 100;
pub const MAX_ALIASES: usize = 8;
pub const MAX_SPELLING: usize = 50;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct Entry {
    pub term: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Merge {
    pub entries: Vec<Entry>,
    /// Terms in `entries` that Verenu now owns.
    pub synced_terms: Vec<String>,
    /// Verenu entries that did not fit T3's entry limit.
    pub skipped: usize,
}

fn key(spelling: &str) -> String {
    spelling.trim().to_lowercase()
}

fn fits(spelling: &str) -> bool {
    let trimmed = spelling.trim();
    !trimmed.is_empty() && trimmed.encode_utf16().count() <= MAX_SPELLING
}

/// Builds one T3 entry from a Verenu term and its known mistranscriptions.
/// Returns `None` when the term cannot be represented in T3.
pub fn entry<'a>(term: &str, mistakes: impl IntoIterator<Item = &'a str>) -> Option<Entry> {
    if !fits(term) {
        return None;
    }
    let term = term.trim().to_string();
    let mut seen = HashSet::from([key(&term)]);
    let aliases = mistakes
        .into_iter()
        .filter(|alias| fits(alias))
        .filter(|alias| seen.insert(key(alias)))
        .map(|alias| alias.trim().to_string())
        .take(MAX_ALIASES)
        .collect();
    Some(Entry { term, aliases })
}

/// Stable identity of the Verenu side, used to skip the network when nothing
/// changed since the last successful sync.
pub fn fingerprint(wanted: &[Entry]) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(wanted).unwrap_or_default())
    )
}

/// Replaces the entries Verenu wrote last time with the current ones.
/// Entries added in T3 are preserved, and win any spelling collision.
pub fn merge(existing: &[Entry], previously_synced: &[String], wanted: &[Entry]) -> Merge {
    let owned: HashSet<String> = previously_synced.iter().map(|term| key(term)).collect();
    let mut entries: Vec<Entry> = existing
        .iter()
        .filter(|entry| !owned.contains(&key(&entry.term)))
        .cloned()
        .collect();
    let mut used: HashSet<String> = entries
        .iter()
        .flat_map(|entry| std::iter::once(&entry.term).chain(&entry.aliases))
        .map(|spelling| key(spelling))
        .collect();
    let mut synced_terms = Vec::new();
    let mut skipped = 0;
    for item in wanted {
        if used.contains(&key(&item.term)) {
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            skipped += 1;
            continue;
        }
        used.insert(key(&item.term));
        let aliases = item
            .aliases
            .iter()
            .filter(|alias| used.insert(key(alias)))
            .take(MAX_ALIASES)
            .cloned()
            .collect();
        synced_terms.push(item.term.clone());
        entries.push(Entry {
            term: item.term.clone(),
            aliases,
        });
    }
    Merge {
        entries,
        synced_terms,
        skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(term: &str, aliases: &[&str]) -> Entry {
        Entry {
            term: term.into(),
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
        }
    }

    #[test]
    fn entry_trims_dedupes_and_caps_aliases() {
        let many: Vec<String> = (0..12).map(|index| format!("alias {index}")).collect();
        let built = entry(
            "  Verenu ",
            ["verenu", " Vreenu ", "vreenu"]
                .into_iter()
                .chain(many.iter().map(String::as_str)),
        )
        .unwrap();
        assert_eq!(built.term, "Verenu");
        assert_eq!(built.aliases.len(), MAX_ALIASES);
        assert_eq!(built.aliases[0], "Vreenu");
        assert!(entry("", []).is_none());
        assert!(entry(&"x".repeat(MAX_SPELLING + 1), []).is_none());
        assert!(entry(&"x".repeat(MAX_SPELLING), []).is_some());
        // T3 counts UTF-16 units, so an emoji costs two.
        assert!(entry(&"😀".repeat(MAX_SPELLING / 2 + 1), []).is_none());
    }

    #[test]
    fn merge_keeps_t3_entries_and_replaces_previous_verenu_entries() {
        let existing = [
            item("Kubernetes", &["cooper netties"]),
            item("Verenu", &["old"]),
        ];
        let wanted = [item("Omarchy", &["omar key"]), item("Hyprland", &[])];
        let merged = merge(&existing, &["verenu".into()], &wanted);
        assert_eq!(
            merged.entries,
            vec![
                item("Kubernetes", &["cooper netties"]),
                item("Omarchy", &["omar key"]),
                item("Hyprland", &[]),
            ]
        );
        assert_eq!(merged.synced_terms, ["Omarchy", "Hyprland"]);
        assert_eq!(merged.skipped, 0);
    }

    #[test]
    fn merge_lets_t3_win_collisions_and_never_creates_duplicate_spellings() {
        let existing = [item("Kubernetes", &["k eight s"])];
        let wanted = [
            item("kubernetes", &["cube"]),
            item("Cube", &["k eight s", "queue b"]),
        ];
        let merged = merge(&existing, &[], &wanted);
        assert_eq!(
            merged.entries,
            vec![
                item("Kubernetes", &["k eight s"]),
                item("Cube", &["queue b"])
            ]
        );
        assert_eq!(merged.synced_terms, ["Cube"]);
        let keys: Vec<String> = merged
            .entries
            .iter()
            .flat_map(|entry| std::iter::once(&entry.term).chain(&entry.aliases))
            .map(|spelling| key(spelling))
            .collect();
        assert_eq!(keys.len(), keys.iter().collect::<HashSet<_>>().len());
    }

    #[test]
    fn merge_respects_the_entry_limit_and_reports_overflow() {
        let existing: Vec<Entry> = (0..98)
            .map(|index| item(&format!("native {index}"), &[]))
            .collect();
        let wanted: Vec<Entry> = (0..5)
            .map(|index| item(&format!("term {index}"), &[]))
            .collect();
        let merged = merge(&existing, &[], &wanted);
        assert_eq!(merged.entries.len(), MAX_ENTRIES);
        assert_eq!(merged.synced_terms, ["term 0", "term 1"]);
        assert_eq!(merged.skipped, 3);
    }

    #[test]
    fn removing_a_term_in_verenu_removes_it_from_t3_on_next_merge() {
        let first = merge(&[], &[], &[item("Omarchy", &[]), item("Hyprland", &[])]);
        let second = merge(
            &first.entries,
            &first.synced_terms,
            &[item("Hyprland", &[])],
        );
        assert_eq!(second.entries, vec![item("Hyprland", &[])]);
    }

    #[test]
    fn fingerprint_changes_with_content() {
        let a = fingerprint(&[item("A", &[])]);
        assert_eq!(a, fingerprint(&[item("A", &[])]));
        assert_ne!(a, fingerprint(&[item("A", &["b"])]));
    }
}
