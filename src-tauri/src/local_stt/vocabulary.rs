//! Immutable recording-local hints. Only canonical terms from the captured
//! Context enter this snapshot; corrections and other Contexts never do.
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Vocabulary(Arc<[String]>);

impl std::fmt::Debug for Vocabulary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vocabulary")
            .field("term_count", &self.0.len())
            .finish()
    }
}

impl Vocabulary {
    pub fn from_terms<'a>(terms: impl IntoIterator<Item = &'a str>) -> Self {
        let mut seen = HashSet::new();
        let terms = terms
            .into_iter()
            .filter_map(|term| {
                let term: String = term
                    .trim()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(100)
                    .collect();
                if term.is_empty() || !seen.insert(term.to_lowercase()) {
                    None
                } else {
                    Some(term)
                }
            })
            .take(100)
            .collect::<Vec<_>>();
        Self(terms.into())
    }

    pub fn for_context(db: &crate::data::db::Db, context_id: i64) -> anyhow::Result<Self> {
        let entries = crate::data::db::query_dictionary_for_context(db, context_id)?;
        Ok(Self::from_terms(
            entries.iter().map(|entry| entry.term.as_str()),
        ))
    }

    pub fn terms(&self) -> &[String] {
        &self.0
    }

    /// UTF-8 byte bound keeps byte-level BPE within 160 prompt tokens even
    /// for scripts whose individual characters span several bytes.
    pub fn whisper_prompt(&self) -> Option<String> {
        let mut prompt = String::new();
        for term in self.terms().iter().take(64) {
            let separator = if prompt.is_empty() { "" } else { ", " };
            if prompt.len() + separator.len() + term.len() > 160 {
                continue;
            }
            prompt.push_str(separator);
            prompt.push_str(term);
        }
        (!prompt.is_empty()).then_some(prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_context_never_contains_other_contexts_or_mistake_strings() {
        let db = crate::data::db::open(":memory:").unwrap();
        let a = crate::data::db::insert_context_returning(
            &db, "Speech A", None, None, None, None, false,
        )
        .unwrap();
        let b = crate::data::db::insert_context_returning(
            &db, "Speech B", None, None, None, None, false,
        )
        .unwrap();
        crate::data::db::insert_dictionary_entry_returning(
            &db,
            "Verenu",
            Some("wrongword"),
            Some(a.id),
        )
        .unwrap();
        crate::data::db::insert_dictionary_entry_returning(&db, "OtherContext", None, Some(b.id))
            .unwrap();
        let captured = Vocabulary::for_context(&db, a.id).unwrap();
        crate::data::db::insert_dictionary_entry_returning(&db, "Later", None, Some(a.id)).unwrap();
        assert_eq!(captured.terms(), &["Verenu"]);
        assert_eq!(
            Vocabulary::for_context(&db, b.id).unwrap().terms(),
            &["OtherContext"]
        );
        assert!(!format!("{captured:?}").contains("Verenu"));
    }
    #[test]
    fn canonical_hints_are_bounded_deduplicated_and_immutable() {
        let mut input = vec![
            " Verenu ".to_string(),
            "verenu".into(),
            "\n\t".into(),
            "語".repeat(200),
        ];
        input.extend((0..200).map(|i| format!("Term{i}")));
        let snapshot = Vocabulary::from_terms(input.iter().map(String::as_str));
        input.clear();
        assert_eq!(snapshot.terms().len(), 100);
        assert_eq!(snapshot.terms()[0], "Verenu");
        assert_eq!(snapshot.terms()[1].chars().count(), 100);
        assert!(snapshot.whisper_prompt().unwrap().len() <= 160);
        assert!(Vocabulary::default().whisper_prompt().is_none());
    }
}
