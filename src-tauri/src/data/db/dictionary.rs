//! Dictionary entries, auto-learn events/candidates, and pending corrections.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::*;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DictionaryCorrection {
    /// Persistent identity of the correction mapping. This is deliberately
    /// separate from `DictionaryEntry::id`: the latter identifies the shared
    /// canonical term and must never be used as a context-scoped rejection
    /// target.
    pub id: i64,
    pub dictionary_id: i64,
    pub context_id: i64,
    pub mistake: String,
    pub auto_learned: bool,
    pub correction_count: i64,
    pub confidence_tier: String,
    pub last_seen_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DictionaryEntry {
    pub id: i64,
    pub term: String,
    pub mistake: Option<String>,
    pub auto_learned: bool,
    pub correction_count: i64,
    pub confidence_tier: String,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    /// Context-effective correction mappings. The legacy Dictionary view
    /// leaves this empty and uses `mistake`; context materialization fills it
    /// with the child mappings effective in that one context. Keeping both
    /// fields lets old IPC consumers continue to render a flattened row while
    /// the pipeline/rejection path uses typed mapping identities.
    #[serde(default)]
    pub corrections: Vec<DictionaryCorrection>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AutoLearnEvent {
    pub id: i64,
    pub event_type: String,
    pub reason_code: String,
    pub context_id: Option<i64>,
    pub app_context: String,
    pub mistake_hash: String,
    pub correction_hash: String,
    pub confidence: f64,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AutoLearnStatusSummary {
    pub monitors_started: i64,
    pub anchor_misses: i64,
    pub low_confidence_rejections: i64,
    pub promotions: i64,
    pub duplicate_monitor_skips: i64,
    pub timeout_finishes: i64,
}

#[derive(Debug)]
struct LegacyDictionaryFields {
    term: String,
    mistake: Option<String>,
    auto_learned: bool,
    correction_count: i64,
    confidence_tier: String,
    last_seen_at: Option<String>,
    created_at: String,
}

struct CorrectionMappingSeed<'a> {
    mistake: Option<&'a str>,
    auto_learned: bool,
    correction_count: i64,
    confidence_tier: &'a str,
    last_seen_at: Option<&'a str>,
}

pub struct AutoLearnEventFields<'a> {
    pub event_type: &'a str,
    pub reason_code: &'a str,
    pub app_context: &'a str,
    pub mistake_hash: &'a str,
    pub correction_hash: &'a str,
    pub confidence: f64,
}

type CorrectionTransferRow = (i64, i64, String, bool, i64, String, Option<String>);

pub fn query_dictionary(db: &Db) -> Result<Vec<DictionaryEntry>> {
    let conn = lock_conn(db)?;
    query_dictionary_conn(&conn, None)
}

pub fn query_dictionary_for_context(db: &Db, context_id: i64) -> Result<Vec<DictionaryEntry>> {
    let conn = lock_conn(db)?;
    query_dictionary_conn(&conn, Some(context_id))
}

fn query_dictionary_conn(
    conn: &rusqlite::Connection,
    context_id: Option<i64>,
) -> Result<Vec<DictionaryEntry>> {
    let canonical_sql = if context_id.is_some() {
        "SELECT d.id, d.term, d.mistake, d.auto_learned, d.correction_count,
                d.confidence_tier, d.last_seen_at, d.created_at
           FROM dictionary d
           INNER JOIN dictionary_contexts dc ON dc.dictionary_id = d.id
          WHERE dc.context_id = ?1
          ORDER BY d.created_at DESC"
    } else {
        "SELECT id, term, mistake, auto_learned, correction_count, confidence_tier,
                last_seen_at, created_at
           FROM dictionary
          WHERE ?1 IS NULL
          ORDER BY created_at DESC"
    };
    let canonical_rows: Vec<(i64, LegacyDictionaryFields)> = conn
        .prepare(canonical_sql)?
        .query_map(params![context_id], |row| {
            Ok((
                row.get(0)?,
                LegacyDictionaryFields {
                    term: row.get(1)?,
                    mistake: row.get(2)?,
                    auto_learned: row.get::<_, i64>(3)? != 0,
                    correction_count: row.get(4)?,
                    confidence_tier: row.get(5)?,
                    last_seen_at: row.get(6)?,
                    created_at: row.get(7)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let correction_sql = if context_id.is_some() {
        "SELECT id, dictionary_id, context_id, mistake, auto_learned,
                correction_count, confidence_tier, last_seen_at, created_at
           FROM dictionary_corrections
          WHERE context_id = ?1
          ORDER BY dictionary_id, id"
    } else {
        "SELECT id, dictionary_id, context_id, mistake, auto_learned,
                correction_count, confidence_tier, last_seen_at, created_at
           FROM dictionary_corrections
          WHERE ?1 IS NULL
          ORDER BY dictionary_id, context_id, id"
    };
    let correction_rows = conn
        .prepare(correction_sql)?
        .query_map(params![context_id], |row| {
            Ok(DictionaryCorrection {
                id: row.get(0)?,
                dictionary_id: row.get(1)?,
                context_id: row.get(2)?,
                mistake: row.get(3)?,
                auto_learned: row.get::<_, i64>(4)? != 0,
                correction_count: row.get(5)?,
                confidence_tier: row.get(6)?,
                last_seen_at: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut corrections_by_dictionary: HashMap<i64, Vec<DictionaryCorrection>> = HashMap::new();
    for correction in correction_rows {
        corrections_by_dictionary
            .entry(correction.dictionary_id)
            .or_default()
            .push(correction);
    }

    Ok(canonical_rows
        .into_iter()
        .map(|(id, mut legacy)| {
            let corrections = corrections_by_dictionary.remove(&id).unwrap_or_default();
            // A scoped query must never fall back to the old global
            // projection.  v26 migrates it into child rows and clears
            // `dictionary.mistake`; ignoring the projection here also
            // keeps a stale value from an interrupted/remote legacy write
            // from leaking a correction learned in another Context.
            if context_id.is_some() {
                legacy.mistake = None;
                legacy.auto_learned = false;
                legacy.correction_count = 0;
                legacy.confidence_tier = "manual".to_string();
                legacy.last_seen_at = None;
            }
            materialize_dictionary_entry(id, legacy, corrections)
        })
        .collect())
}

fn materialize_dictionary_entry(
    id: i64,
    legacy: LegacyDictionaryFields,
    corrections: Vec<DictionaryCorrection>,
) -> DictionaryEntry {
    if corrections.is_empty() {
        // This fallback is only for a pre-v26/partially repaired row. v26
        // clears the global projection, and all writes after v26 populate the
        // child table, so a healthy database cannot leak it into a Context.
        return DictionaryEntry {
            id,
            term: legacy.term,
            mistake: legacy.mistake,
            auto_learned: legacy.auto_learned,
            correction_count: legacy.correction_count,
            confidence_tier: legacy.confidence_tier,
            last_seen_at: legacy.last_seen_at,
            created_at: legacy.created_at,
            corrections,
        };
    }

    let mut seen = HashSet::new();
    let mistake = corrections
        .iter()
        .filter(|correction| seen.insert(correction.mistake.to_lowercase()))
        .map(|correction| correction.mistake.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let auto_learned = corrections.iter().any(|correction| correction.auto_learned);
    let correction_count = corrections
        .iter()
        .map(|correction| correction.correction_count)
        .sum();
    let confidence_tier = corrections
        .iter()
        .filter(|correction| correction.auto_learned)
        .max_by_key(|correction| confidence_tier_rank(&correction.confidence_tier))
        .map(|correction| correction.confidence_tier.clone())
        .unwrap_or_else(|| "manual".to_string());
    let last_seen_at = corrections
        .iter()
        .filter_map(|correction| correction.last_seen_at.clone())
        .max();

    DictionaryEntry {
        id,
        term: legacy.term,
        mistake: (!mistake.is_empty()).then_some(mistake),
        auto_learned,
        correction_count,
        confidence_tier,
        last_seen_at,
        created_at: legacy.created_at,
        corrections,
    }
}

fn confidence_tier_rank(tier: &str) -> u8 {
    match tier {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

/// Move correction mappings along with their canonical dictionary assignment
/// when a Context is deleted.
///
/// Context deletion has always moved vocabulary assignments to Everywhere.
/// The child mapping must follow that move, but a direct `UPDATE` can collide
/// with an existing Everywhere mapping. An exact same-term/same-variant row is
/// merged (manual metadata wins); a different canonical term already using the
/// same variant wins deterministically and the deleted-Context row is dropped
/// rather than creating an ambiguous substitution.
pub fn move_dictionary_corrections_conn(
    conn: &rusqlite::Connection,
    from_context_id: i64,
    to_context_id: i64,
) -> Result<usize> {
    if from_context_id == to_context_id {
        return Ok(0);
    }

    let source_rows: Vec<CorrectionTransferRow> = conn
        .prepare(
            "SELECT id, dictionary_id, mistake, auto_learned, correction_count,
                    confidence_tier, last_seen_at
               FROM dictionary_corrections
              WHERE context_id = ?1
              ORDER BY id",
        )?
        .query_map(params![from_context_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get::<_, i64>(3)? != 0,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut changed = 0;
    for (
        id,
        dictionary_id,
        mistake,
        source_auto_learned,
        source_count,
        source_tier,
        source_last_seen_at,
    ) in source_rows
    {
        let exact_target: Option<(i64, bool)> = conn
            .query_row(
                "SELECT id, auto_learned
                   FROM dictionary_corrections
                  WHERE context_id = ?1 AND dictionary_id = ?2 AND mistake = ?3
                  LIMIT 1",
                params![to_context_id, dictionary_id, mistake],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .optional()?;

        if let Some((target_id, target_auto_learned)) = exact_target {
            // A manual mapping is authoritative. If the source mapping is
            // manual and the target was automatic, transfer that authority;
            // otherwise retain the existing target row and only accumulate
            // automatic evidence.
            if !source_auto_learned && target_auto_learned {
                conn.execute(
                    "UPDATE dictionary_corrections
                        SET auto_learned = 0,
                            correction_count = ?2,
                            confidence_tier = 'manual',
                            last_seen_at = ?3
                      WHERE id = ?1",
                    params![target_id, source_count, source_last_seen_at],
                )?;
            } else if source_auto_learned && target_auto_learned {
                let (target_count, target_tier, target_last_seen_at):
                    (i64, String, Option<String>) = conn.query_row(
                        "SELECT correction_count, confidence_tier, last_seen_at
                           FROM dictionary_corrections
                          WHERE id = ?1",
                        params![target_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )?;
                let last_seen_at = match (target_last_seen_at, source_last_seen_at) {
                    (Some(target), Some(source)) => Some(target.max(source)),
                    (target, source) => target.or(source),
                };
                let tier =
                    if confidence_tier_rank(&source_tier) > confidence_tier_rank(&target_tier) {
                        source_tier
                    } else {
                        target_tier
                    };
                conn.execute(
                    "UPDATE dictionary_corrections
                        SET correction_count = ?2,
                            confidence_tier = ?3,
                            last_seen_at = ?4
                      WHERE id = ?1",
                    params![target_id, target_count + source_count, tier, last_seen_at],
                )?;
            }
            conn.execute(
                "DELETE FROM dictionary_corrections WHERE id = ?1",
                params![id],
            )?;
            changed += 1;
            continue;
        }

        // The unique Context/variant index cannot represent two canonical
        // terms for the same spelling. Keep the pre-existing Everywhere row;
        // this is the same deterministic conflict policy used during legacy
        // migration and default seeding.
        let variant_conflict: bool = conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM dictionary_corrections
                  WHERE context_id = ?1 AND mistake = ?2 AND dictionary_id != ?3
             )",
            params![to_context_id, mistake, dictionary_id],
            |row| row.get(0),
        )?;
        if variant_conflict {
            conn.execute(
                "DELETE FROM dictionary_corrections WHERE id = ?1",
                params![id],
            )?;
            changed += 1;
            continue;
        }

        changed += conn.execute(
            "UPDATE dictionary_corrections SET context_id = ?2 WHERE id = ?1",
            params![id, to_context_id],
        )?;
    }

    Ok(changed)
}

/// Remove all correction mappings for one canonical item from one Context.
///
/// This is the connection-level operation used by the Context assignment
/// "remove" path. Removing only `dictionary_contexts` would leave an orphaned
/// child mapping that could reappear if the canonical item is assigned again,
/// and would leave stale evidence eligible for promotion.
pub fn remove_dictionary_corrections_for_context_conn(
    conn: &rusqlite::Connection,
    context_id: i64,
    dictionary_id: i64,
) -> Result<usize> {
    let term: Option<String> = conn
        .query_row(
            "SELECT term FROM dictionary WHERE id = ?1",
            params![dictionary_id],
            |row| row.get(0),
        )
        .optional()?;
    let mappings: Vec<(String, String)> = conn
        .prepare(
            "SELECT c.mistake, d.term
               FROM dictionary_corrections c
               INNER JOIN dictionary d ON d.id = c.dictionary_id
              WHERE c.context_id = ?1 AND c.dictionary_id = ?2",
        )?
        .query_map(params![context_id, dictionary_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    for (mistake, term) in mappings {
        for variant in dictionary_mistake_variants(&mistake) {
            purge_auto_learn_evidence_for_pair_conn(conn, context_id, variant, &term)?;
        }
    }
    // Evidence can outlive its promoted child row after a rejection or a
    // partially completed import. Removing the canonical assignment must
    // purge that evidence even when no mapping row remains to enumerate it.
    if let Some(term) = term.as_deref() {
        purge_auto_learn_evidence_for_term_conn(conn, context_id, term)?;
    }

    Ok(conn.execute(
        "DELETE FROM dictionary_corrections
           WHERE context_id = ?1 AND dictionary_id = ?2",
        params![context_id, dictionary_id],
    )?)
}

fn purge_auto_learn_evidence_for_pair_conn(
    conn: &rusqlite::Connection,
    context_id: i64,
    mistake: &str,
    term: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM pending_corrections
           WHERE context_id = ?1
             AND lower(wrong_word) = lower(?2)
             AND lower(correct_word) = lower(?3)",
        params![context_id, mistake, term],
    )?;
    conn.execute(
        "DELETE FROM auto_learn_candidates
           WHERE context_id = ?1
             AND lower(wrong_word) = lower(?2)
             AND lower(correct_word) = lower(?3)",
        params![context_id, mistake, term],
    )?;
    Ok(())
}

fn purge_auto_learn_evidence_for_term_conn(
    conn: &rusqlite::Connection,
    context_id: i64,
    term: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM pending_corrections
           WHERE context_id = ?1 AND lower(correct_word) = lower(?2)",
        params![context_id, term],
    )?;
    conn.execute(
        "DELETE FROM auto_learn_candidates
           WHERE context_id = ?1 AND lower(correct_word) = lower(?2)",
        params![context_id, term],
    )?;
    Ok(())
}

fn purge_auto_learn_evidence_for_mistake_list_conn(
    conn: &rusqlite::Connection,
    context_id: i64,
    mistake: Option<&str>,
    term: &str,
) -> Result<()> {
    for variant in mistake.into_iter().flat_map(dictionary_mistake_variants) {
        purge_auto_learn_evidence_for_pair_conn(conn, context_id, variant, term)?;
    }
    Ok(())
}

/// Returns the comma-separated mistranscription variants in a dictionary
/// field. Variants are compared after trimming and case-folding, but their
/// stored spelling is preserved for display and prompt evidence.
pub fn dictionary_mistake_variants(mistake: &str) -> impl Iterator<Item = &str> {
    mistake
        .split(',')
        .map(str::trim)
        .filter(|variant| !variant.is_empty())
}

fn normalized_mistake_variants(mistake: Option<&str>) -> HashSet<String> {
    mistake
        .into_iter()
        .flat_map(dictionary_mistake_variants)
        .map(|variant| variant.to_lowercase())
        .collect()
}

/// Rejects a dictionary entry when one of its mistranscription variants is
/// already mapped to a different term in the same context group. This is
/// intentionally checked at the database boundary because entries can enter a
/// context through create, edit, or assignment paths.
pub fn check_dictionary_mistake_conflicts(
    conn: &rusqlite::Connection,
    context_id: i64,
    dictionary_id: Option<i64>,
    mistake: Option<&str>,
) -> Result<()> {
    let candidate_variants = normalized_mistake_variants(mistake);
    if candidate_variants.is_empty() {
        return Ok(());
    }

    // v26 makes child rows the only Context-aware source of corrections.
    // Deliberately do not consult `dictionary.mistake`: it is retained only
    // as a legacy compatibility column and a stale value must not block or
    // leak a mapping in a different Context.
    let mut stmt = conn.prepare(
        "SELECT d.id, d.term, c.mistake
           FROM dictionary_corrections c
           INNER JOIN dictionary d ON d.id = c.dictionary_id
          WHERE c.context_id = ?1
            AND (?2 IS NULL OR d.id != ?2)",
    )?;
    let rows = stmt.query_map(params![context_id, dictionary_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;

    for row in rows {
        let (_id, term, existing_mistake) = row?;
        for variant in dictionary_mistake_variants(existing_mistake.as_deref().unwrap_or("")) {
            if candidate_variants.contains(&variant.to_lowercase()) {
                anyhow::bail!(
                    "Often mistranscribed as \"{variant}\" already belongs to \"{term}\" in this context"
                );
            }
        }
    }

    Ok(())
}

fn normalized_mistake_list(mistake: Option<&str>) -> Vec<String> {
    let mut seen = HashSet::new();
    mistake
        .into_iter()
        .flat_map(dictionary_mistake_variants)
        .filter(|variant| seen.insert(variant.to_lowercase()))
        .map(str::to_owned)
        .collect()
}

/// Inserts the context-owned child mappings for one canonical term.  The
/// caller must perform the context conflict check first.  A mapping row is
/// intentionally one variant, not the legacy comma-separated field, because
/// the row id is the rejection/sync identity.
fn insert_correction_mappings_conn(
    conn: &rusqlite::Connection,
    context_id: i64,
    dictionary_id: i64,
    seed: CorrectionMappingSeed<'_>,
) -> Result<usize> {
    let variants = normalized_mistake_list(seed.mistake);
    let mut inserted = 0;
    for (index, variant) in variants.iter().enumerate() {
        let changed = conn.execute(
            "INSERT INTO dictionary_corrections
               (uuid, context_id, dictionary_id, mistake, auto_learned,
                correction_count, confidence_tier, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                Uuid::new_v4().to_string(),
                context_id,
                dictionary_id,
                variant,
                seed.auto_learned as i64,
                if index == 0 { seed.correction_count } else { 0 },
                seed.confidence_tier,
                seed.last_seen_at,
            ],
        )?;
        inserted += changed;
    }
    Ok(inserted)
}

#[cfg(test)]
pub fn insert_dictionary_entry(db: &Db, term: &str, mistake: Option<&str>) -> Result<()> {
    insert_dictionary_entry_returning(db, term, mistake, None)?;
    Ok(())
}

/// Creates a dictionary entry, or — when `context_id` names a specific
/// (non-Everywhere) context and the term already exists elsewhere — links the
/// existing entry into that context instead of failing on the term's UNIQUE
/// constraint. Only errors on a duplicate when the entry is already assigned
/// to that same context; a term already scoped to a *different* context is
/// fair game to also assign here, since a term can belong to more than one
/// context at once (see `dictionary_contexts`).
///
/// `context_id: None` (used by the legacy standalone Dictionary page and bulk
/// import) keeps the original strict behavior: duplicate terms always fail,
/// and new entries land in Everywhere.
pub fn insert_dictionary_entry_returning(
    db: &Db,
    term: &str,
    mistake: Option<&str>,
    context_id: Option<i64>,
) -> Result<CreatedRecordMeta> {
    let normalized_term = require_nonempty_trimmed("Term", term)?;
    let normalized_mistake = normalize_optional_trimmed(mistake);
    validate_char_limit("Term", &normalized_term, DICTIONARY_ENTRY_CHAR_LIMIT)?;
    if let Some(m) = normalized_mistake.as_deref() {
        validate_char_limit("Often mistranscribed as", m, DICTIONARY_ENTRY_CHAR_LIMIT)?;
    }

    // Insert, assign, and read last_insert_rowid under a single lock to prevent
    // another thread's insert racing between the two acquisitions.
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let everywhere_id = ensure_everywhere_context_conn(&tx)?;
    let target_context = context_id.unwrap_or(everywhere_id);
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM contexts WHERE id = ?1)",
        params![target_context],
        |row| row.get(0),
    )?;
    if !exists {
        anyhow::bail!("Context {target_context} was not found");
    }
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM dictionary WHERE term = ?1",
            params![normalized_term],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        if context_id.is_none() {
            anyhow::bail!("\"{normalized_term}\" is already in the dictionary");
        }
        let already_in_context: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM dictionary_contexts WHERE context_id = ?1 AND dictionary_id = ?2)",
            params![target_context, id],
            |row| row.get(0),
        )?;
        if already_in_context {
            anyhow::bail!("\"{normalized_term}\" is already in this context");
        }
        check_dictionary_mistake_conflicts(
            &tx,
            target_context,
            Some(id),
            normalized_mistake.as_deref(),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id) VALUES (?1, ?2)",
            params![target_context, id],
        )?;
        // The legacy projection is no longer authoritative. Clear it
        // when an existing row is touched so later legacy consumers
        // cannot observe a stale correction that has no scoped owner.
        tx.execute(
            "UPDATE dictionary SET mistake = NULL WHERE id = ?1 AND mistake IS NOT NULL",
            params![id],
        )?;
        insert_correction_mappings_conn(
            &tx,
            target_context,
            id,
            CorrectionMappingSeed {
                mistake: normalized_mistake.as_deref(),
                auto_learned: false,
                correction_count: 0,
                confidence_tier: "manual",
                last_seen_at: None,
            },
        )?;
        // Keep the legacy global field as a compatibility projection for
        // the standalone pre-Context sync path. Context-aware consumers
        // use dictionary_corrections as the source of truth and ignore it.
        if target_context == everywhere_id {
            tx.execute(
                "UPDATE dictionary SET mistake = ?2 WHERE id = ?1",
                params![id, normalized_mistake],
            )?;
        }
        purge_auto_learn_evidence_for_mistake_list_conn(
            &tx,
            target_context,
            normalized_mistake.as_deref(),
            &normalized_term,
        )?;
        let created_at = tx.query_row(
            "SELECT created_at FROM dictionary WHERE id=?1",
            params![id],
            |r| r.get(0),
        )?;
        tx.commit()?;
        return Ok(CreatedRecordMeta { id, created_at });
    }

    check_dictionary_mistake_conflicts(&tx, target_context, None, normalized_mistake.as_deref())?;
    tx.execute(
        "INSERT INTO dictionary (term, mistake, confidence_tier, last_seen_at) VALUES (?1, NULL, 'manual', datetime('now'))",
        params![normalized_term],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id) VALUES (?1, ?2)",
        params![target_context, id],
    )?;
    insert_correction_mappings_conn(
        &tx,
        target_context,
        id,
        CorrectionMappingSeed {
            mistake: normalized_mistake.as_deref(),
            auto_learned: false,
            correction_count: 0,
            confidence_tier: "manual",
            last_seen_at: None,
        },
    )?;
    // See the compatibility projection note above. This is only populated for
    // Everywhere, never for a targeted Context.
    if target_context == everywhere_id {
        tx.execute(
            "UPDATE dictionary SET mistake = ?2 WHERE id = ?1",
            params![id, normalized_mistake],
        )?;
    }
    purge_auto_learn_evidence_for_mistake_list_conn(
        &tx,
        target_context,
        normalized_mistake.as_deref(),
        &normalized_term,
    )?;
    let created_at = tx.query_row(
        "SELECT created_at FROM dictionary WHERE id=?1",
        params![id],
        |r| r.get(0),
    )?;
    tx.commit()?;
    Ok(CreatedRecordMeta { id, created_at })
}

/// Ensures the dictionary's "Verenu" entry (if any) lists every known
/// mistranscription observed in practice from local speech-to-text models —
/// small local STT models get their own product name wrong almost every
/// time, unlike cloud Whisper models, which get a spelling hint baked into
/// their transcription prompt (see `TRANSCRIPTION_GLOSSARY`) that local
/// engines have no equivalent for (`transcribe-rs`'s `TranscribeOptions` has
/// no prompt/vocabulary field at all). `mistake` is comma-separated (see
/// `parse_dictionary_mistakes` in `data::dictionary`), same as a snippet's
/// multi-trigger field.
///
/// Two cases, handled differently:
/// - **An entry for "Verenu" already exists** (a prior run of this
///   function, or — the real bug this fixes — the user's own manual entry
///   predating this feature entirely, e.g. a hand-added `Verenu -> Vernu`).
///   Any of the known variants missing from its `mistake` list are merged
///   in; anything already there (including variants this function doesn't
///   know about) is left untouched. This branch is safe and idempotent to
///   run on every launch, unconditionally — it only ever adds, so it also
///   self-heals a database like this one where `INSERT OR IGNORE` had
///   silently lost every known variant to a `UNIQUE` conflict against a
///   pre-existing row, without clobbering what the user already had.
/// - **No entry exists.** Create one with the full known list — but only
///   the first time ever, gated on the `seeded_defaults` marker table, so a
///   user who deletes the entry entirely doesn't see it recreated on the
///   next launch. (Not `PRAGMA user_version`: that's schema.rs's own
///   migration counter, and claiming a version number here for a one-off
///   data seed would collide with any future structural migration needing
///   the same number.)
///
/// Deliberately NOT part of the generic migration chain in `schema::open` —
/// that function is called directly by `open(":memory:")` all over the test
/// suite (fixtures, unit tests), and seeding real product data into every
/// ephemeral test database would silently change dictionary counts/contents
/// those tests don't expect. This is called once per launch, explicitly,
/// only where the real user database opens (`main.rs`).
pub fn seed_default_dictionary_entries(db: &Db) -> Result<()> {
    const MARKER: &str = "verenu_dictionary_v1";
    // Zarinu is evidenced live: Cohere (local STT) consistently rendered
    // "Verenu" with a leading Z across multiple dictations in the same
    // session (confirmed by the speaker directly: "Cohere is trying to say
    // it with a Z sometimes"). Berenu/Ferenu/Werenu/Verinu extend the same
    // voiced/voiceless and vowel-substitution confusions the existing list
    // already covers (B/V, F/V, W/V, e/i) — invented non-words, so they
    // carry no real-word collision risk the way a plausible English word
    // would. Varineu is also evidenced live: Cohere transcribed "named
    // Verenu" as "named Varineu" verbatim in both raw and cleaned text.
    const KNOWN_VARIANTS: [&str; 15] = [
        "Varinu", "Verena", "Virinu", "Varino", "Varinew", "Varina", "Verminu", "Varinian",
        "Marino", "Zarinu", "Berenu", "Ferenu", "Werenu", "Verinu", "Varineu",
    ];

    let mut conn = lock_conn(db)?;
    let existing: Option<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM dictionary WHERE term = 'Verenu' LIMIT 1")?;
        let mut rows = stmt.query([])?;
        match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        }
    };

    match existing {
        Some(id) => {
            let tx = conn.transaction()?;
            let everywhere_id = ensure_everywhere_context_conn(&tx)?;
            tx.execute(
                "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
                 VALUES (?1, ?2)",
                params![everywhere_id, id],
            )?;
            for known in KNOWN_VARIANTS {
                let already_mapped: bool = tx.query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM dictionary_corrections
                        WHERE context_id = ?1 AND dictionary_id = ?2 AND mistake = ?3
                     )",
                    params![everywhere_id, id, known],
                    |row| row.get(0),
                )?;
                if already_mapped {
                    continue;
                }
                let conflicting: bool = tx.query_row(
                    "SELECT EXISTS(
                       SELECT 1 FROM dictionary_corrections
                        WHERE context_id = ?1 AND mistake = ?2 AND dictionary_id != ?3
                     )",
                    params![everywhere_id, known, id],
                    |row| row.get(0),
                )?;
                if !conflicting {
                    tx.execute(
                        "INSERT INTO dictionary_corrections
                           (uuid, context_id, dictionary_id, mistake, confidence_tier)
                         VALUES (?1, ?2, ?3, ?4, 'manual')",
                        params![Uuid::new_v4().to_string(), everywhere_id, id, known],
                    )?;
                }
            }
            tx.execute(
                "UPDATE dictionary SET mistake = NULL WHERE id = ?1",
                params![id],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO seeded_defaults (key) VALUES (?1)",
                params![MARKER],
            )?;
            tx.commit()?;
        }
        None => {
            let already_seeded: i64 = conn.query_row(
                "SELECT COUNT(*) FROM seeded_defaults WHERE key = ?1",
                params![MARKER],
                |r| r.get(0),
            )?;
            if already_seeded > 0 {
                return Ok(());
            }
            let mut conn = conn;
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO dictionary (term, mistake, confidence_tier) VALUES ('Verenu', NULL, 'manual')",
                [],
            )?;
            let id = tx.last_insert_rowid();
            let everywhere_id = ensure_everywhere_context_conn(&tx)?;
            tx.execute(
                "INSERT INTO dictionary_contexts (context_id, dictionary_id) VALUES (?1, ?2)",
                params![everywhere_id, id],
            )?;
            insert_correction_mappings_conn(
                &tx,
                everywhere_id,
                id,
                CorrectionMappingSeed {
                    mistake: Some(&KNOWN_VARIANTS.join(", ")),
                    auto_learned: false,
                    correction_count: 0,
                    confidence_tier: "manual",
                    last_seen_at: None,
                },
            )?;
            tx.execute(
                "INSERT INTO seeded_defaults (key) VALUES (?1)",
                params![MARKER],
            )?;
            tx.commit()?;
        }
    }
    Ok(())
}

/// Inserts a dictionary entry restored from a backup file. Takes an
/// already-locked connection so a caller doing many inserts (bulk import)
/// can wrap them all in one transaction instead of locking per row.
pub fn insert_dictionary_entry_from_backup_conn(
    conn: &rusqlite::Connection,
    term: &str,
    mistake: Option<&str>,
    auto_learned: bool,
    confidence_tier: &str,
    correction_count: i64,
) -> Result<()> {
    let normalized_term = require_nonempty_trimmed("Term", term)?;
    let normalized_mistake = normalize_optional_trimmed(mistake);
    validate_char_limit("Term", &normalized_term, DICTIONARY_ENTRY_CHAR_LIMIT)?;
    if let Some(m) = normalized_mistake.as_deref() {
        validate_char_limit("Often mistranscribed as", m, DICTIONARY_ENTRY_CHAR_LIMIT)?;
    }
    conn.execute(
        "INSERT INTO dictionary (term, mistake, auto_learned, correction_count, confidence_tier, last_seen_at) \
         VALUES (?1, NULL, ?2, ?3, ?4, datetime('now'))",
        params![normalized_term, auto_learned as i64, correction_count, confidence_tier],
    )?;
    let id = conn.last_insert_rowid();
    let everywhere_id = ensure_everywhere_context_conn(conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id) VALUES (?1, ?2)",
        params![everywhere_id, id],
    )?;
    insert_correction_mappings_conn(
        conn,
        everywhere_id,
        id,
        CorrectionMappingSeed {
            mistake: normalized_mistake.as_deref(),
            auto_learned,
            correction_count,
            confidence_tier,
            last_seen_at: None,
        },
    )?;
    Ok(())
}

#[cfg(test)]
pub fn insert_dictionary_entry_auto_learned(
    db: &Db,
    term: &str,
    mistake: Option<&str>,
    confidence_tier: &str,
) -> Result<bool> {
    insert_dictionary_entry_auto_learned_for_context(
        db,
        EVERYWHERE_CONTEXT_ID,
        term,
        mistake,
        confidence_tier,
    )
}

#[cfg(test)]
pub fn insert_dictionary_entry_auto_learned_for_context(
    db: &Db,
    context_id: i64,
    term: &str,
    mistake: Option<&str>,
    confidence_tier: &str,
) -> Result<bool> {
    let normalized_term = require_nonempty_trimmed("Term", term)?;
    let normalized_mistake = normalize_optional_trimmed(mistake);
    validate_char_limit("Term", &normalized_term, DICTIONARY_ENTRY_CHAR_LIMIT)?;
    if let Some(mistake) = normalized_mistake.as_deref() {
        validate_char_limit(
            "Often mistranscribed as",
            mistake,
            DICTIONARY_ENTRY_CHAR_LIMIT,
        )?;
    }

    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM contexts WHERE id = ?1)",
        params![context_id],
        |row| row.get(0),
    )?;
    if !exists {
        anyhow::bail!("Context {context_id} was not found");
    }
    let id: i64 = match tx
        .query_row(
            "SELECT id FROM dictionary WHERE term = ?1",
            params![normalized_term],
            |r| r.get(0),
        )
        .optional()?
    {
        Some(id) => id,
        None => {
            tx.execute(
                "INSERT INTO dictionary (term, mistake, auto_learned, correction_count, confidence_tier)
                 VALUES (?1, NULL, 1, 0, ?2)",
                params![normalized_term, confidence_tier],
            )?;
            tx.last_insert_rowid()
        }
    };
    // Manual correction mappings are authoritative only in their owning
    // Context. A manual mapping for the same canonical term elsewhere must
    // not poison this Context, but an automatic mapping must not overwrite a
    // manual mapping here either.
    let manual_mapping_exists: bool = tx.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM dictionary_corrections
            WHERE context_id = ?1 AND dictionary_id = ?2 AND auto_learned = 0
         )",
        params![context_id, id],
        |row| row.get(0),
    )?;
    if manual_mapping_exists {
        tx.commit()?;
        return Ok(false);
    }
    tx.execute(
        "UPDATE dictionary SET mistake = NULL WHERE id = ?1 AND mistake IS NOT NULL",
        params![id],
    )?;
    check_dictionary_mistake_conflicts(&tx, context_id, Some(id), normalized_mistake.as_deref())?;
    tx.execute(
        "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id) VALUES (?1, ?2)",
        params![context_id, id],
    )?;

    let mut changed = false;
    for variant in normalized_mistake_list(normalized_mistake.as_deref()) {
        let existing: Option<(i64, bool)> = tx
            .query_row(
                "SELECT id, auto_learned FROM dictionary_corrections
                  WHERE context_id = ?1 AND dictionary_id = ?2 AND mistake = ?3",
                params![context_id, id, variant],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .optional()?;
        match existing {
            Some((correction_id, true)) => {
                tx.execute(
                    "UPDATE dictionary_corrections
                        SET correction_count = correction_count + 1,
                            confidence_tier = ?1,
                            last_seen_at = datetime('now')
                      WHERE id = ?2",
                    params![confidence_tier, correction_id],
                )?;
                changed = true;
            }
            Some((_correction_id, false)) => {}
            None => {
                tx.execute(
                    "INSERT INTO dictionary_corrections
                       (uuid, context_id, dictionary_id, mistake, auto_learned,
                        correction_count, confidence_tier, last_seen_at)
                     VALUES (?1, ?2, ?3, ?4, 1, 1, ?5, datetime('now'))",
                    params![
                        Uuid::new_v4().to_string(),
                        context_id,
                        id,
                        variant,
                        confidence_tier
                    ],
                )?;
                changed = true;
            }
        }
    }
    tx.commit()?;
    Ok(changed)
}

/// Context-aware event writer. The legacy wrapper above remains for old
/// telemetry callers; new monitor paths should always provide the immutable
/// originating Context id.
#[expect(dead_code, reason = "Consumed by the Context-aware monitor in the stacked runtime change")]
pub fn log_auto_learn_event_for_context(
    db: &Db,
    context_id: i64,
    event: AutoLearnEventFields<'_>,
) -> Result<()> {
    log_auto_learn_event_with_context(
        db,
        Some(context_id),
        event,
    )
}

fn log_auto_learn_event_with_context(
    db: &Db,
    context_id: Option<i64>,
    event: AutoLearnEventFields<'_>,
) -> Result<()> {
    let conn = lock_conn(db)?;
    conn.execute(
        "INSERT INTO auto_learn_events
         (event_type, reason_code, context_id, app_context, mistake_hash, correction_hash, confidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event.event_type,
            event.reason_code,
            context_id,
            event.app_context,
            event.mistake_hash,
            event.correction_hash,
            event.confidence
        ],
    )?;
    Ok(())
}

/// Records a confidence observation for a (wrong, correct) pair and returns
/// the running average confidence across all observations of that pair in one
/// Context. Context is part of the natural key; observations from another
/// Context cannot contribute to this average.
pub fn upsert_auto_learn_candidate_for_context(
    db: &Db,
    context_id: i64,
    wrong: &str,
    correct: &str,
    confidence: f64,
) -> Result<f64> {
    let conn = lock_conn(db)?;
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM contexts WHERE id = ?1)",
        params![context_id],
        |row| row.get(0),
    )?;
    if !exists {
        anyhow::bail!("Context {context_id} was not found");
    }
    conn.query_row(
        "INSERT INTO auto_learn_candidates
         (context_id, wrong_word, correct_word, confidence_sum, confidence_avg, seen_count, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?4, 1, datetime('now'))
         ON CONFLICT(context_id, wrong_word, correct_word) DO UPDATE SET
           confidence_sum = auto_learn_candidates.confidence_sum + excluded.confidence_sum,
           seen_count = auto_learn_candidates.seen_count + 1,
           confidence_avg = (auto_learn_candidates.confidence_sum + excluded.confidence_sum) / (auto_learn_candidates.seen_count + 1),
           last_seen_at = datetime('now')
         RETURNING confidence_avg",
        params![context_id, wrong, correct, confidence],
        |r| r.get(0),
    )
    .map_err(Into::into)
}

/// Compatibility wrapper for the legacy monitor API. The Context-aware
/// runtime uses [`upsert_auto_learn_candidate_for_context`]; callers that do
/// not yet capture a Context retain the historical Everywhere scope until
/// they are migrated.
/// Outcome of an [`auto_learn_promote_for_context`] attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoLearnPromoteResult {
    /// The pair was promoted to the dictionary (or its existing auto-learned
    /// entry was reinforced) and the candidate's `promoted_at` gate claimed.
    Promoted,
    /// Pending-correction count hasn't reached the threshold yet. The pending
    /// row was recorded so the next session counts toward the threshold.
    BelowThreshold { pending_count: i64 },
    /// A manual dictionary entry for the same term blocks auto-learn. The
    /// candidate gate was NOT claimed, so removing the manual entry later lets
    /// the pair be learned anew.
    Blocked,
    /// A concurrent monitor (or a rejection that already purged the candidate)
    /// won the promotion claim first. The pair must not promote again.
    AlreadyPromoted,
}

/// Atomically records a pending correction, checks the promotion threshold,
/// claims the candidate's `promoted_at` gate, and promotes the pair into the
/// dictionary — all in one transaction under the single DB lock.
///
/// Auto-learn monitors are intentionally concurrent (one per dictation), and
/// two monitors can observe the same `(wrong, correct)` pair within the same
/// 2-day window. Previously the pending-count read and the dictionary upsert
/// were separate lock acquisitions, so two monitors could BOTH pass the
/// threshold and BOTH "promote" the same pair — double `promoted` events and
/// an inflated `correction_count`. A rejection monitor could also delete the
/// entry/candidate in the gap, and the in-flight promotion would re-create the
/// rejected entry. `promoted_at` is the single promotion gate: it is claimed
/// here (atomically, `IS NULL` guard) and only cleared by the rejection /
/// manual-delete paths, which purge the candidate row entirely.
pub fn auto_learn_promote_for_context(
    db: &Db,
    context_id: i64,
    wrong: &str,
    correct: &str,
    confidence_tier: &str,
    pending_retention_days: i64,
    threshold: i64,
) -> Result<AutoLearnPromoteResult> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let context_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM contexts WHERE id = ?1)",
        params![context_id],
        |row| row.get(0),
    )?;
    if !context_exists {
        anyhow::bail!("Context {context_id} was not found");
    }

    // Rejection and manual-removal paths purge the candidate row. Check that
    // the still-unclaimed, Context-scoped candidate exists before adding a
    // pending observation. Without this guard, a promotion that starts after
    // rejection could recreate fresh pending evidence, then discover that its
    // candidate was gone and leave stale state that immediately re-promotes.
    let candidate_available: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM auto_learn_candidates
              WHERE context_id = ?1 AND wrong_word = ?2 AND correct_word = ?3
                AND promoted_at IS NULL
         )",
        params![context_id, wrong, correct],
        |r| r.get(0),
    )?;
    if !candidate_available {
        tx.commit()?;
        return Ok(AutoLearnPromoteResult::AlreadyPromoted);
    }

    tx.execute(
        "INSERT INTO pending_corrections (context_id, wrong_word, correct_word)
         VALUES (?1, ?2, ?3)",
        params![context_id, wrong, correct],
    )?;

    let pending_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM pending_corrections
         WHERE context_id = ?1 AND wrong_word = ?2 AND correct_word = ?3
           AND created_at >= datetime('now', ?4)",
        params![
            context_id,
            wrong,
            correct,
            format!("-{} days", pending_retention_days.max(1))
        ],
        |r| r.get(0),
    )?;

    if pending_count < threshold.max(1) {
        tx.commit()?;
        return Ok(AutoLearnPromoteResult::BelowThreshold { pending_count });
    }

    // A manual entry must block auto-learn WITHOUT claiming the candidate, so
    // that deleting the manual entry later allows the pair to be learned anew.
    let manual_exists: i64 = tx.query_row(
        "SELECT COUNT(*)
           FROM dictionary_corrections c
           INNER JOIN dictionary d ON d.id = c.dictionary_id
          WHERE c.context_id = ?1
            AND d.term = ?2
            AND c.auto_learned = 0",
        params![context_id, correct],
        |r| r.get(0),
    )?;
    if manual_exists > 0 {
        tx.commit()?;
        return Ok(AutoLearnPromoteResult::Blocked);
    }

    // A Context may already use this mistranscription for another canonical
    // term. Treat that as a normal AutoLearn block, before claiming the
    // candidate gate, so the candidate remains eligible only if the competing
    // mapping is later removed. Database failures still propagate normally.
    let existing_dictionary_id: Option<i64> = tx
        .query_row(
            "SELECT id FROM dictionary WHERE term = ?1",
            params![correct],
            |r| r.get(0),
        )
        .optional()?;
    if let Err(error) =
        check_dictionary_mistake_conflicts(&tx, context_id, existing_dictionary_id, Some(wrong))
    {
        if error.to_string().starts_with("Often mistranscribed as ") {
            tx.commit()?;
            return Ok(AutoLearnPromoteResult::Blocked);
        }
        return Err(error);
    }

    // Atomic claim on the candidate. 0 rows means the candidate is already
    // promoted (a concurrent monitor won the race) or was purged by a
    // rejection / manual delete — either way this pair must not promote again.
    let claimed = tx.execute(
        "UPDATE auto_learn_candidates
         SET promoted_at = datetime('now')
         WHERE context_id = ?1 AND wrong_word = ?2 AND correct_word = ?3
           AND promoted_at IS NULL",
        params![context_id, wrong, correct],
    )?;
    if claimed == 0 {
        tx.commit()?;
        return Ok(AutoLearnPromoteResult::AlreadyPromoted);
    }

    // Ensure the shared canonical identity exists. The actual correction
    // mapping below is Context-owned, so an existing canonical row from
    // another Context is not a conflict and a different mistake can coexist.
    tx.execute(
        "INSERT INTO dictionary
           (term, mistake, auto_learned, correction_count, confidence_tier, last_seen_at)
         VALUES (?1, NULL, 1, 0, ?2, datetime('now'))
         ON CONFLICT(term) DO NOTHING",
        params![correct, confidence_tier],
    )?;

    let dictionary_id: i64 = tx.query_row(
        "SELECT id FROM dictionary WHERE term = ?1",
        params![correct],
        |r| r.get(0),
    )?;
    tx.execute(
        "UPDATE dictionary SET mistake = NULL WHERE id = ?1 AND mistake IS NOT NULL",
        params![dictionary_id],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id) VALUES (?1, ?2)",
        params![context_id, dictionary_id],
    )?;

    tx.execute(
        "INSERT INTO dictionary_corrections
           (uuid, context_id, dictionary_id, mistake, auto_learned,
            correction_count, confidence_tier, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, 1, 1, ?5, datetime('now'))
         ON CONFLICT(context_id, dictionary_id, mistake) DO UPDATE SET
           auto_learned = 1,
           correction_count = dictionary_corrections.correction_count + 1,
           confidence_tier = excluded.confidence_tier,
           last_seen_at = datetime('now')",
        params![
            Uuid::new_v4().to_string(),
            context_id,
            dictionary_id,
            wrong,
            confidence_tier
        ],
    )?;

    tx.commit()?;
    Ok(AutoLearnPromoteResult::Promoted)
}

/// Compatibility wrapper for legacy callers that do not carry the resolved
/// Context. New code must use [`auto_learn_promote_for_context`] so evidence
/// and persistent mappings retain their originating Context.
pub fn get_auto_learn_status_summary(db: &Db) -> Result<AutoLearnStatusSummary> {
    let conn = lock_conn(db)?;
    let count_by = |event_type: &str, reason_code: &str| -> Result<i64> {
        conn.query_row(
            "SELECT COUNT(*) FROM auto_learn_events WHERE event_type = ?1 AND reason_code = ?2",
            params![event_type, reason_code],
            |r| r.get(0),
        )
        .map_err(Into::into)
    };
    let monitors_started: i64 = conn.query_row(
        "SELECT COUNT(*) FROM auto_learn_events WHERE event_type = 'monitor'",
        [],
        |r| r.get(0),
    )?;
    let promotions: i64 = conn.query_row(
        "SELECT COUNT(*) FROM auto_learn_events WHERE event_type = 'promotion' AND reason_code = 'promoted'",
        [],
        |r| r.get(0),
    )?;
    Ok(AutoLearnStatusSummary {
        monitors_started,
        anchor_misses: count_by("anchor", "anchor_miss")?,
        low_confidence_rejections: count_by("candidate", "low_confidence")?,
        promotions,
        duplicate_monitor_skips: count_by("monitor", "duplicate_skip")?,
        timeout_finishes: count_by("monitor", "timeout")?,
    })
}

pub fn get_recent_auto_learn_activity(db: &Db, limit: i64) -> Result<Vec<AutoLearnEvent>> {
    let conn = lock_conn(db)?;
    let mut stmt = conn.prepare(
        "SELECT id, event_type, reason_code, context_id, app_context, mistake_hash, correction_hash, confidence, created_at
         FROM auto_learn_events
         ORDER BY created_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit.max(1)], |r| {
            Ok(AutoLearnEvent {
                id: r.get(0)?,
                event_type: r.get(1)?,
                reason_code: r.get(2)?,
                context_id: r.get(3)?,
                app_context: r.get(4)?,
                mistake_hash: r.get(5)?,
                correction_hash: r.get(6)?,
                confidence: r.get(7)?,
                created_at: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
pub fn insert_pending_correction(db: &Db, wrong: &str, correct: &str) -> Result<()> {
    insert_pending_correction_for_context(db, EVERYWHERE_CONTEXT_ID, wrong, correct)
}

/// Test helper for inserting evidence into an explicit Context. Production
/// promotion records pending evidence inside its own transaction; keeping the
/// scoped helper test-only avoids exposing a write API for transient state.
#[cfg(test)]
pub fn insert_pending_correction_for_context(
    db: &Db,
    context_id: i64,
    wrong: &str,
    correct: &str,
) -> Result<()> {
    let conn = lock_conn(db)?;
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM contexts WHERE id = ?1)",
        params![context_id],
        |row| row.get(0),
    )?;
    if !exists {
        anyhow::bail!("Context {context_id} was not found");
    }
    conn.execute(
        "INSERT INTO pending_corrections (context_id, wrong_word, correct_word)
         VALUES (?1, ?2, ?3)",
        params![context_id, wrong, correct],
    )?;
    Ok(())
}

#[cfg(test)]
pub fn count_pending_corrections_recent(
    db: &Db,
    context_id: i64,
    wrong: &str,
    correct: &str,
    max_age_days: i64,
) -> Result<i64> {
    count_pending_corrections_recent_for_context(db, context_id, wrong, correct, max_age_days)
}

#[cfg(test)]
pub fn count_pending_corrections_recent_for_context(
    db: &Db,
    context_id: i64,
    wrong: &str,
    correct: &str,
    max_age_days: i64,
) -> Result<i64> {
    let conn = lock_conn(db)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_corrections \
         WHERE context_id=?1 AND wrong_word=?2 AND correct_word=?3 \
         AND created_at >= datetime('now', ?4)",
        params![
            context_id,
            wrong,
            correct,
            format!("-{} days", max_age_days.max(1))
        ],
        |r| r.get(0),
    )?;
    Ok(count)
}

pub fn prune_pending_corrections(db: &Db, max_age_days: i64) -> Result<usize> {
    let conn = lock_conn(db)?;
    let changed = conn.execute(
        "DELETE FROM pending_corrections \
         WHERE created_at < datetime('now', ?1)",
        params![format!("-{} days", max_age_days.max(1))],
    )?;
    Ok(changed)
}

/// Bounded retention for the auto-learn bookkeeping tables, so they cannot
/// grow without limit on long-lived installs: monitor/promotion audit events
/// are pruned after 30 days, and candidate rows that have not been seen in 90
/// days (promoted or not) are dropped — a candidate that is still alive after
/// 90 days of inactivity has outlived its learning window. Rejection and
/// manual-delete already purge their own rows; this catches everything else.
/// Called at startup alongside the cleanup-cache prune.
pub fn prune_auto_learn_retention(db: &Db) -> Result<usize> {
    let conn = lock_conn(db)?;
    let events = conn.execute(
        "DELETE FROM auto_learn_events \
         WHERE created_at < datetime('now', '-30 days')",
        [],
    )?;
    let candidates = conn.execute(
        "DELETE FROM auto_learn_candidates \
         WHERE last_seen_at < datetime('now', '-90 days')",
        [],
    )?;
    Ok(events + candidates)
}

pub fn update_dictionary_entry_for_context(
    db: &Db,
    context_id: i64,
    id: i64,
    term: &str,
    mistake: Option<&str>,
) -> Result<()> {
    let normalized_term = require_nonempty_trimmed("Term", term)?;
    let normalized_mistake = normalize_optional_trimmed(mistake);
    validate_char_limit("Term", &normalized_term, DICTIONARY_ENTRY_CHAR_LIMIT)?;
    if let Some(mistake) = normalized_mistake.as_deref() {
        validate_char_limit(
            "Often mistranscribed as",
            mistake,
            DICTIONARY_ENTRY_CHAR_LIMIT,
        )?;
    }

    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    update_dictionary_entry_for_context_conn(
        &tx,
        context_id,
        id,
        &normalized_term,
        normalized_mistake.as_deref(),
    )?;
    tx.commit()?;
    Ok(())
}

fn update_dictionary_entry_for_context_conn(
    conn: &rusqlite::Connection,
    context_id: i64,
    id: i64,
    normalized_term: &str,
    normalized_mistake: Option<&str>,
) -> Result<()> {
    let context_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM contexts WHERE id = ?1)",
        params![context_id],
        |row| row.get(0),
    )?;
    if !context_exists {
        anyhow::bail!("Context {context_id} was not found");
    }
    let assigned: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM dictionary_contexts
                        WHERE context_id = ?1 AND dictionary_id = ?2)",
        params![context_id, id],
        |row| row.get(0),
    )?;
    if !assigned {
        anyhow::bail!("Dictionary entry {id} is not assigned to this context");
    }
    check_dictionary_mistake_conflicts(conn, context_id, Some(id), normalized_mistake)?;

    let changed = conn.execute(
        "UPDATE dictionary SET term = ?2, mistake = NULL WHERE id = ?1",
        params![id, normalized_term],
    )?;
    require_row_changed(changed, "Dictionary entry", id)?;

    // An explicit edit is manual authority for this Context. Remove any
    // automatic mappings/evidence only in this scope before replacing them.
    purge_auto_learn_evidence_for_dictionary_conn(conn, context_id, id)?;
    conn.execute(
        "DELETE FROM dictionary_corrections WHERE context_id = ?1 AND dictionary_id = ?2",
        params![context_id, id],
    )?;
    insert_correction_mappings_conn(
        conn,
        context_id,
        id,
        CorrectionMappingSeed {
            mistake: normalized_mistake,
            auto_learned: false,
            correction_count: 0,
            confidence_tier: "manual",
            last_seen_at: None,
        },
    )?;
    // Lower-stack compatibility: the old dictionary sync payload only knows
    // about dictionary.mistake. The Context-aware runtime ignores this
    // projection and reads the child mapping instead.
    conn.execute(
        "UPDATE dictionary
            SET mistake = CASE WHEN EXISTS(
                SELECT 1 FROM contexts WHERE id = ?2 AND is_everywhere = 1
            ) THEN ?3 ELSE NULL END
          WHERE id = ?1",
        params![id, context_id, normalized_mistake],
    )?;
    purge_auto_learn_evidence_for_mistake_list_conn(
        conn,
        context_id,
        normalized_mistake,
        normalized_term,
    )?;
    Ok(())
}

/// Legacy Dictionary-page edits have no Context argument. Refuse to apply one
/// edit across divergent Context mappings rather than silently overwriting
/// them; the Contexts surface should call `update_dictionary_entry_for_context`.
pub fn update_dictionary_entry(db: &Db, id: i64, term: &str, mistake: Option<&str>) -> Result<()> {
    let conn = lock_conn(db)?;
    let context_ids: Vec<i64> = conn
        .prepare("SELECT context_id FROM dictionary_contexts WHERE dictionary_id = ?1 ORDER BY context_id")?
        .query_map(params![id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let context_id = if context_ids.is_empty() {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM dictionary WHERE id = ?1)",
            params![id],
            |row| row.get(0),
        )?;
        if !exists {
            anyhow::bail!("Dictionary entry {id} was not found");
        }
        // The lower-stack legacy sync path can deliver a canonical dictionary
        // row before its Context aggregate. Preserve the old Dictionary-page
        // behavior by treating an unassigned existing row as Everywhere.
        let everywhere_id: i64 = conn.query_row(
            "SELECT id FROM contexts WHERE is_everywhere = 1 ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
             VALUES (?1, ?2)",
            params![everywhere_id, id],
        )?;
        everywhere_id
    } else if context_ids.len() > 1 {
        anyhow::bail!(
            "Dictionary entry {id} is shared by multiple contexts; edit it from the active Context"
        );
    } else {
        context_ids[0]
    };
    drop(conn);
    update_dictionary_entry_for_context(db, context_id, id, term, mistake)
}

/// Remove a canonical dictionary assignment from one Context without deleting
/// the shared canonical row or any of its mappings in other Contexts.
///
/// This is the Contexts-surface counterpart to the legacy global delete. An
/// automatically-created canonical row is removed only when this was its last
/// assignment and no correction mapping remains anywhere.
#[expect(dead_code, reason = "Consumed by the Contexts library command in the stacked runtime change")]
pub fn remove_dictionary_entry_from_context(
    db: &Db,
    context_id: i64,
    dictionary_id: i64,
) -> Result<()> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let assigned: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM dictionary_contexts
              WHERE context_id = ?1 AND dictionary_id = ?2
         )",
        params![context_id, dictionary_id],
        |row| row.get(0),
    )?;
    if !assigned {
        anyhow::bail!("Dictionary entry {dictionary_id} is not assigned to this context");
    }
    remove_dictionary_corrections_for_context_conn(&tx, context_id, dictionary_id)?;
    tx.execute(
        "DELETE FROM dictionary_contexts WHERE context_id = ?1 AND dictionary_id = ?2",
        params![context_id, dictionary_id],
    )?;
    cleanup_orphaned_auto_dictionary_conn(&tx, context_id, dictionary_id)?;
    tx.commit()?;
    Ok(())
}

/// Move one canonical assignment from one Context to another. Unlike adding a
/// shared item, moving is an explicit transfer: its Context-owned correction
/// mappings follow the assignment, while any unpromoted evidence in the source
/// Context is discarded because it has no safe destination.
#[expect(dead_code, reason = "Consumed by the Contexts library command in the stacked runtime change")]
pub fn move_dictionary_entry_to_context(
    db: &Db,
    dictionary_id: i64,
    source_context_id: i64,
    target_context_id: i64,
) -> Result<()> {
    if source_context_id == target_context_id {
        return Ok(());
    }

    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    for context_id in [source_context_id, target_context_id] {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM contexts WHERE id = ?1)",
            params![context_id],
            |row| row.get(0),
        )?;
        if !exists {
            anyhow::bail!("Context {context_id} was not found");
        }
    }

    let source_assigned: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM dictionary_contexts
              WHERE context_id = ?1 AND dictionary_id = ?2
         )",
        params![source_context_id, dictionary_id],
        |row| row.get(0),
    )?;
    if !source_assigned {
        anyhow::bail!("Dictionary entry {dictionary_id} is not assigned to the source context");
    }
    let target_assigned: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM dictionary_contexts
              WHERE context_id = ?1 AND dictionary_id = ?2
         )",
        params![target_context_id, dictionary_id],
        |row| row.get(0),
    )?;
    if target_assigned {
        anyhow::bail!("Dictionary entry {dictionary_id} is already assigned to the target context");
    }

    // Preflight the target so a move cannot transfer some variants and then
    // fail on a later collision. A wrong spelling may belong to only one
    // canonical term in a Context.
    let source_mistakes: Vec<String> = tx
        .prepare(
            "SELECT mistake FROM dictionary_corrections
              WHERE context_id = ?1 AND dictionary_id = ?2 ORDER BY id",
        )?
        .query_map(params![source_context_id, dictionary_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for mistake in &source_mistakes {
        let conflict: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM dictionary_corrections
                  WHERE context_id = ?1 AND mistake = ?2 AND dictionary_id != ?3
             )",
            params![target_context_id, mistake, dictionary_id],
            |row| row.get(0),
        )?;
        if conflict {
            anyhow::bail!(
                "Often mistranscribed as \"{mistake}\" already belongs to another entry in the target context"
            );
        }
    }

    tx.execute(
        "INSERT INTO dictionary_contexts (context_id, dictionary_id) VALUES (?1, ?2)",
        params![target_context_id, dictionary_id],
    )?;
    tx.execute(
        "UPDATE dictionary_corrections
            SET context_id = ?2
          WHERE context_id = ?1 AND dictionary_id = ?3",
        params![source_context_id, target_context_id, dictionary_id],
    )?;
    let term: Option<String> = tx
        .query_row(
            "SELECT term FROM dictionary WHERE id = ?1",
            params![dictionary_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(term) = term.as_deref() {
        purge_auto_learn_evidence_for_term_conn(&tx, source_context_id, term)?;
    }
    tx.execute(
        "DELETE FROM dictionary_contexts WHERE context_id = ?1 AND dictionary_id = ?2",
        params![source_context_id, dictionary_id],
    )?;
    tx.commit()?;
    Ok(())
}

fn purge_auto_learn_evidence_for_dictionary_conn(
    conn: &rusqlite::Connection,
    context_id: i64,
    dictionary_id: i64,
) -> Result<usize> {
    let pairs: Vec<(String, String)> = conn
        .prepare(
            "SELECT c.mistake, d.term
               FROM dictionary_corrections c
               INNER JOIN dictionary d ON d.id = c.dictionary_id
              WHERE c.context_id = ?1 AND c.dictionary_id = ?2 AND c.auto_learned = 1",
        )?
        .query_map(params![context_id, dictionary_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut deleted = 0;
    for (mistake, term) in pairs {
        deleted += conn.execute(
            "DELETE FROM pending_corrections
              WHERE context_id = ?1 AND lower(wrong_word) = lower(?2)
                AND lower(correct_word) = lower(?3)",
            params![context_id, mistake, term],
        )?;
        deleted += conn.execute(
            "DELETE FROM auto_learn_candidates
              WHERE context_id = ?1 AND lower(wrong_word) = lower(?2)
                AND lower(correct_word) = lower(?3)",
            params![context_id, mistake, term],
        )?;
    }
    Ok(deleted)
}

pub fn delete_dictionary_entry(db: &Db, id: i64) -> Result<()> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;

    // Capture the canonical term and every Context-owned correction before
    // deleting. The old global `dictionary.mistake` projection is NULL after
    // v26, so looking only at that column would leave stale scoped evidence.
    let info: Option<(String, Option<String>, i64)> = tx
        .query_row(
            "SELECT term, mistake, auto_learned FROM dictionary WHERE id=?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let assigned_contexts: Vec<i64> = tx
        .prepare("SELECT context_id FROM dictionary_contexts WHERE dictionary_id = ?1")?
        .query_map(params![id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut evidence: Vec<(i64, String, String)> = tx
        .prepare(
            "SELECT c.context_id, c.mistake, d.term
               FROM dictionary_corrections c
               INNER JOIN dictionary d ON d.id = c.dictionary_id
              WHERE c.dictionary_id = ?1",
        )?
        .query_map(params![id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if let Some((term, Some(mistake), _)) = &info {
        if evidence.is_empty() {
            evidence.extend(
                assigned_contexts
                    .iter()
                    .copied()
                    .map(|context_id| (context_id, mistake.clone(), term.clone())),
            );
        }
    }

    // A v26 canonical row can legitimately have no child mapping after a
    // context-scoped rejection or partial import. Purge by canonical term as
    // well, otherwise stale evidence could recreate the row after this delete.
    if let Some((term, _, _)) = &info {
        for context_id in &assigned_contexts {
            purge_auto_learn_evidence_for_term_conn(&tx, *context_id, term)?;
        }
    }

    for (context_id, mistake, term) in evidence {
        for variant in dictionary_mistake_variants(&mistake) {
            tx.execute(
                "DELETE FROM pending_corrections
                   WHERE context_id = ?1
                     AND lower(wrong_word) = lower(?2)
                     AND lower(correct_word) = lower(?3)",
                params![context_id, variant, term],
            )?;
            tx.execute(
                "DELETE FROM auto_learn_candidates
                   WHERE context_id = ?1
                     AND lower(wrong_word) = lower(?2)
                     AND lower(correct_word) = lower(?3)",
                params![context_id, variant, term],
            )?;
        }
    }

    tx.execute(
        "DELETE FROM dictionary_contexts WHERE dictionary_id = ?1",
        params![id],
    )?;
    let changed = tx.execute("DELETE FROM dictionary WHERE id=?1", params![id])?;
    require_row_changed(changed, "Dictionary entry", id)?;

    tx.commit()?;
    Ok(())
}

// Remove a canonical row only when rejection has left an automatically
// created entry with neither a mapping nor another Context assignment.
pub(crate) fn cleanup_orphaned_auto_dictionary_conn(
    conn: &rusqlite::Connection,
    context_id: i64,
    dictionary_id: i64,
) -> Result<()> {
    let orphan: Option<(String, bool)> = conn
        .query_row(
            "SELECT term, auto_learned = 1
               FROM dictionary
              WHERE id = ?1
                AND NOT EXISTS (
                    SELECT 1 FROM dictionary_corrections
                     WHERE dictionary_id = ?1
                )
                AND NOT EXISTS (
                    SELECT 1 FROM dictionary_contexts
                     WHERE dictionary_id = ?1 AND context_id != ?2
                )",
            params![dictionary_id, context_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((term, auto_learned)) = orphan else {
        return Ok(());
    };
    if !auto_learned {
        return Ok(());
    }

    // There is no remaining mapping that can identify a narrower pair, so
    // clear every stale evidence row for this canonical term in the rejected
    // Context before removing its last assignment. This is intentionally
    // scoped: another Context may still have an independently learned pair.
    conn.execute(
        "DELETE FROM pending_corrections
           WHERE context_id = ?1 AND lower(correct_word) = lower(?2)",
        params![context_id, term],
    )?;
    conn.execute(
        "DELETE FROM auto_learn_candidates
           WHERE context_id = ?1 AND lower(correct_word) = lower(?2)",
        params![context_id, term],
    )?;
    conn.execute(
        "DELETE FROM dictionary_contexts WHERE dictionary_id = ?1",
        params![dictionary_id],
    )?;
    conn.execute(
        "DELETE FROM dictionary WHERE id = ?1 AND auto_learned = 1",
        params![dictionary_id],
    )?;
    Ok(())
}

/// Delete only automatic correction mappings identified by their persistent
/// child-row IDs in one Context.
///
/// A rejection monitor receives `dictionary_corrections.id`, never the shared
/// `dictionary.id`. The transaction verifies both the Context and the
/// `auto_learned` flag before deleting, then purges the matching pending and
/// candidate rows in that same Context so a rejected mapping cannot
/// immediately re-promote. Manual mappings, other Contexts, and the shared
/// canonical term remain untouched unless the canonical row is an auto-learned
/// orphan with no remaining Context assignment.
pub fn delete_auto_learned_corrections_by_ids(
    db: &Db,
    context_id: i64,
    correction_ids: &[i64],
) -> Result<usize> {
    if correction_ids.is_empty() {
        return Ok(0);
    }

    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let mut deleted = 0;
    let mut dictionary_ids = HashSet::new();
    for correction_id in correction_ids {
        let mapping: Option<(i64, String, String)> = tx
            .query_row(
                "SELECT c.dictionary_id, c.mistake, d.term
                   FROM dictionary_corrections c
                   INNER JOIN dictionary d ON d.id = c.dictionary_id
                  WHERE c.id = ?1 AND c.context_id = ?2 AND c.auto_learned = 1",
                params![correction_id, context_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((dictionary_id, mistake, term)) = mapping else {
            // This also makes repeated rejection notifications idempotent and
            // keeps a manual row with a stale/incorrect target ID protected.
            continue;
        };

        let changed = tx.execute(
            "DELETE FROM dictionary_corrections
               WHERE id = ?1 AND context_id = ?2 AND auto_learned = 1",
            params![correction_id, context_id],
        )?;
        if changed == 0 {
            continue;
        }
        deleted += changed;
        dictionary_ids.insert(dictionary_id);

        // Child rows normally contain one variant, but accepting the legacy
        // comma-separated representation here keeps cleanup correct for a
        // partially migrated database as well.
        for variant in dictionary_mistake_variants(&mistake) {
            tx.execute(
                "DELETE FROM pending_corrections
                   WHERE context_id = ?1
                     AND lower(wrong_word) = lower(?2)
                     AND lower(correct_word) = lower(?3)",
                params![context_id, variant, term],
            )?;
            tx.execute(
                "DELETE FROM auto_learn_candidates
                   WHERE context_id = ?1
                     AND lower(wrong_word) = lower(?2)
                     AND lower(correct_word) = lower(?3)",
                params![context_id, variant, term],
            )?;
        }
    }
    for dictionary_id in dictionary_ids {
        cleanup_orphaned_auto_dictionary_conn(&tx, context_id, dictionary_id)?;
    }

    tx.commit()?;
    Ok(deleted)
}

/// Test-only compatibility wrapper for the legacy rejection caller, whose IDs are
/// canonical dictionary ids and whose historical behavior only ever learned
/// into Everywhere. New callers must use
/// `delete_auto_learned_corrections_by_ids` with child mapping ids and an
/// originating Context.
#[cfg(test)]
pub fn delete_auto_learned_entries_by_ids(db: &Db, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let conn = lock_conn(db)?;
    let everywhere_id: i64 = conn.query_row(
        "SELECT id FROM contexts WHERE is_everywhere = 1 ORDER BY id LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let mapping_ids: Vec<i64> = {
        // rusqlite does not expose a portable array parameter. The small
        // compatibility path can safely inspect each requested id.
        let mut result = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT c.id
               FROM dictionary_corrections c
              WHERE c.context_id = ?1 AND c.dictionary_id = ?2 AND c.auto_learned = 1",
        )?;
        for id in ids {
            let rows = stmt
                .query_map(params![everywhere_id, id], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            result.extend(rows);
        }
        result
    };
    drop(conn);
    delete_auto_learned_corrections_by_ids(db, everywhere_id, &mapping_ids)?;
    Ok(())
}
