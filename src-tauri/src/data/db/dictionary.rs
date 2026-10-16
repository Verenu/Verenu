use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::*;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DictionaryCorrection {
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
    #[serde(default)]
    pub corrections: Vec<DictionaryCorrection>,
}

fn dictionary_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DictionaryEntry> {
    Ok(DictionaryEntry {
        id: row.get(0)?,
        term: row.get(1)?,
        mistake: row.get(2)?,
        auto_learned: row.get::<_, i64>(3)? != 0,
        correction_count: row.get(4)?,
        confidence_tier: row.get(5)?,
        last_seen_at: row.get(6)?,
        created_at: row.get(7)?,
        corrections: Vec::new(),
    })
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
        .prepare_cached(canonical_sql)?
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
    let mut correction_stmt = conn.prepare_cached(correction_sql)?;
    let correction_rows = correction_stmt.query_map(params![context_id], |row| {
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
    })?;
    let mut corrections_by_dictionary: HashMap<i64, Vec<DictionaryCorrection>> = HashMap::new();
    for correction in correction_rows {
        let correction = correction?;
        corrections_by_dictionary
            .entry(correction.dictionary_id)
            .or_default()
            .push(correction);
    }

    Ok(canonical_rows
        .into_iter()
        .map(|(id, mut legacy)| {
            let corrections = corrections_by_dictionary.remove(&id).unwrap_or_default();
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
        .filter_map(|correction| correction.last_seen_at.as_ref())
        .max()
        .cloned();

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
                let (target_count, target_tier, target_last_seen_at): (
                    i64,
                    String,
                    Option<String>,
                ) = conn.query_row(
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
) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM pending_corrections
           WHERE context_id = ?1
             AND lower(wrong_word) = lower(?2)
             AND lower(correct_word) = lower(?3)",
        params![context_id, mistake, term],
    )?;
    let candidates = conn.execute(
        "DELETE FROM auto_learn_candidates
           WHERE context_id = ?1
             AND lower(wrong_word) = lower(?2)
             AND lower(correct_word) = lower(?3)",
        params![context_id, mistake, term],
    )?;
    Ok(deleted + candidates)
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

pub fn check_dictionary_mistake_conflicts(
    conn: &rusqlite::Connection,
    context_id: i64,
    dictionary_id: Option<i64>,
    mistake: Option<&str>,
) -> Result<()> {
    if let Some((variant, term)) =
        find_dictionary_mistake_conflict(conn, context_id, dictionary_id, mistake)?
    {
        anyhow::bail!(
            "Often mistranscribed as \"{variant}\" already belongs to \"{term}\" in this context"
        );
    }
    Ok(())
}

fn find_dictionary_mistake_conflict(
    conn: &rusqlite::Connection,
    context_id: i64,
    dictionary_id: Option<i64>,
    mistake: Option<&str>,
) -> Result<Option<(String, String)>> {
    let candidate_variants = normalized_mistake_variants(mistake);
    if candidate_variants.is_empty() {
        return Ok(None);
    }

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
                return Ok(Some((variant.to_string(), term)));
            }
        }
    }

    Ok(None)
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

pub fn move_dictionary_entry_by_term_to_context(
    db: &Db,
    term: &str,
    target_context_id: i64,
) -> Result<DictionaryEntry> {
    let normalized_term = require_nonempty_trimmed("Term", term)?;
    validate_char_limit("Term", &normalized_term, DICTIONARY_ENTRY_CHAR_LIMIT)?;

    let (dictionary_id, everywhere_id) = {
        let conn = lock_conn(db)?;
        let everywhere_id = ensure_everywhere_context_conn(&conn)?;
        let dictionary_id: i64 = conn
            .query_row(
                "SELECT id FROM dictionary WHERE term = ?1",
                params![normalized_term],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("\"{normalized_term}\" was not found"))?;
        (dictionary_id, everywhere_id)
    };
    if target_context_id == everywhere_id {
        anyhow::bail!("The Everywhere context cannot be the move destination");
    }

    move_dictionary_entry_to_context(db, dictionary_id, everywhere_id, target_context_id)?;

    let conn = lock_conn(db)?;
    Ok(conn.query_row(
        "SELECT id, term, mistake, auto_learned, correction_count, confidence_tier,
                last_seen_at, created_at
         FROM dictionary WHERE id = ?1",
        params![dictionary_id],
        dictionary_entry_from_row,
    )?)
}

pub fn seed_default_dictionary_entries(db: &Db) -> Result<()> {
    const MARKER: &str = "verenu_dictionary_v1";
    const KNOWN_VARIANTS: [&str; 16] = [
        "Varinu", "Verena", "Virinu", "Varino", "Varinew", "Varina", "Verminu", "Varinian",
        "Marino", "Zarinu", "Berenu", "Ferenu", "Werenu", "Verinu", "Varineu", "Verino",
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
            let mut available_variants = Vec::new();
            for known in KNOWN_VARIANTS {
                let conflicting: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM dictionary_corrections
                     WHERE context_id = ?1 AND mistake = ?2)",
                    params![everywhere_id, known],
                    |row| row.get(0),
                )?;
                if !conflicting {
                    available_variants.push(known);
                }
            }
            insert_correction_mappings_conn(
                &tx,
                everywhere_id,
                id,
                CorrectionMappingSeed {
                    mistake: Some(&available_variants.join(", ")),
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

pub fn log_auto_learn_event_for_context(
    db: &Db,
    context_id: i64,
    event: AutoLearnEventFields<'_>,
) -> Result<()> {
    log_auto_learn_event_with_context(db, Some(context_id), event)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoLearnPromoteResult {
    Promoted,
    BelowThreshold { pending_count: i64 },
    Blocked,
    AlreadyPromoted,
}

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

    let existing_dictionary_id: Option<i64> = tx
        .query_row(
            "SELECT id FROM dictionary WHERE term = ?1",
            params![correct],
            |r| r.get(0),
        )
        .optional()?;
    if find_dictionary_mistake_conflict(&tx, context_id, existing_dictionary_id, Some(wrong))?
        .is_some()
    {
        tx.commit()?;
        return Ok(AutoLearnPromoteResult::Blocked);
    }

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
    query_all(
        &conn,
        "SELECT id, event_type, reason_code, context_id, app_context, mistake_hash, correction_hash, confidence, created_at
         FROM auto_learn_events
         ORDER BY created_at DESC
         LIMIT ?1",
        params![limit.max(1)], |r| {
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
        },
    )
}

#[cfg(test)]
pub fn insert_pending_correction(db: &Db, wrong: &str, correct: &str) -> Result<()> {
    insert_pending_correction_for_context(db, EVERYWHERE_CONTEXT_ID, wrong, correct)
}

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
        deleted += purge_auto_learn_evidence_for_pair_conn(conn, context_id, &mistake, &term)?;
    }
    Ok(deleted)
}

pub fn delete_dictionary_entry(db: &Db, id: i64) -> Result<()> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;

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

    if let Some((term, _, _)) = &info {
        for context_id in &assigned_contexts {
            purge_auto_learn_evidence_for_term_conn(&tx, *context_id, term)?;
        }
    }

    for (context_id, mistake, term) in evidence {
        for variant in dictionary_mistake_variants(&mistake) {
            purge_auto_learn_evidence_for_pair_conn(&tx, context_id, variant, &term)?;
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

    purge_auto_learn_evidence_for_term_conn(conn, context_id, &term)?;
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

        for variant in dictionary_mistake_variants(&mistake) {
            purge_auto_learn_evidence_for_pair_conn(&tx, context_id, variant, &term)?;
        }
    }
    for dictionary_id in dictionary_ids {
        cleanup_orphaned_auto_dictionary_conn(&tx, context_id, dictionary_id)?;
    }

    tx.commit()?;
    Ok(deleted)
}

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

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn pair_purge_counts_both_tables_and_preserves_other_evidence() {
        let db = open(":memory:").unwrap();
        let conn = lock_conn(&db).unwrap();
        conn.execute("INSERT INTO contexts (id, name) VALUES (2, 'Other')", [])
            .unwrap();
        for table in ["pending_corrections", "auto_learn_candidates"] {
            for (context_id, mistake, term) in [
                (1, "Wrong", "Term"),
                (1, "WRONG", "TERM"),
                (2, "wrong", "term"),
                (1, "different", "term"),
                (1, "wrong", "other"),
            ] {
                conn.execute(
                    &format!("INSERT INTO {table} (context_id, wrong_word, correct_word) VALUES (?1, ?2, ?3)"),
                    params![context_id, mistake, term],
                ).unwrap();
            }
        }
        assert_eq!(
            purge_auto_learn_evidence_for_pair_conn(&conn, 1, "wrong", "term").unwrap(),
            4
        );
        assert_eq!(
            purge_auto_learn_evidence_for_pair_conn(&conn, 1, "wrong", "term").unwrap(),
            0
        );
        for table in ["pending_corrections", "auto_learn_candidates"] {
            let remaining = query_all(
                &conn,
                &format!("SELECT context_id, wrong_word, correct_word FROM {table} ORDER BY id"),
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap();
            assert_eq!(
                remaining,
                vec![
                    (2, "wrong".into(), "term".into()),
                    (1, "different".into(), "term".into()),
                    (1, "wrong".into(), "other".into()),
                ]
            );
        }
    }
}
