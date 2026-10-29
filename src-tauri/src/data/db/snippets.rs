use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::*;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Snippet {
    pub id: i64,
    pub trigger: String,
    pub expansion: String,
    pub instructions: String,
    pub use_count: i64,
    pub created_at: String,
    /// True when this row is also available through Everywhere inheritance.
    #[serde(default)]
    pub inherited_from_everywhere: bool,
    /// True when the active group has its own assignment for this row.
    #[serde(default)]
    pub assigned_to_context: bool,
}

fn snippet_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Snippet> {
    Ok(Snippet {
        id: row.get(0)?,
        trigger: row.get(1)?,
        expansion: row.get(2)?,
        instructions: row.get(3)?,
        use_count: row.get(4)?,
        created_at: row.get(5)?,
        inherited_from_everywhere: false,
        assigned_to_context: false,
    })
}

fn normalize_snippet(
    trigger: &str,
    expansion: &str,
    instructions: &str,
) -> Result<(String, String, String)> {
    let trigger = require_nonempty_trimmed("Trigger", trigger)?;
    validate_char_limit("Trigger", &trigger, SNIPPET_TRIGGER_CHAR_LIMIT)?;
    let expansion = normalize_multiline(expansion);
    if expansion.is_empty() {
        anyhow::bail!("Expansion cannot be empty");
    }
    validate_char_limit("Expansion", &expansion, SNIPPET_EXPANSION_CHAR_LIMIT)?;
    let instructions = normalize_multiline(instructions);
    validate_char_limit(
        "Cleanup instructions",
        &instructions,
        SNIPPET_INSTRUCTIONS_CHAR_LIMIT,
    )?;
    Ok((trigger, expansion, instructions))
}

#[cfg(test)]
pub fn insert_snippet(db: &Db, trigger: &str, expansion: &str, instructions: &str) -> Result<()> {
    insert_snippet_returning(db, trigger, expansion, instructions, None)?;
    Ok(())
}

pub fn insert_snippet_returning(
    db: &Db,
    trigger: &str,
    expansion: &str,
    instructions: &str,
    context_id: Option<i64>,
) -> Result<CreatedRecordMeta> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let created = insert_snippet_returning_conn(&tx, trigger, expansion, instructions, context_id)?;
    tx.commit()?;
    Ok(created)
}

pub fn insert_snippet_returning_conn(
    conn: &rusqlite::Connection,
    trigger: &str,
    expansion: &str,
    instructions: &str,
    context_id: Option<i64>,
) -> Result<CreatedRecordMeta> {
    let (normalized_trigger, normalized_expansion, normalized_instructions) =
        normalize_snippet(trigger, expansion, instructions)?;

    let everywhere_id = ensure_everywhere_context_conn(conn)?;
    let target_context = context_id.filter(|id| *id != everywhere_id);

    if let Some(target_context) = target_context {
        let existing: Option<(i64, String, String, String)> = conn
            .query_row(
                "SELECT id, expansion, instructions, created_at FROM snippets WHERE trigger = ?1",
                params![normalized_trigger],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        if let Some((id, expansion, instructions, created_at)) = existing {
            let already_in_context: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM snippet_contexts WHERE context_id = ?1 AND snippet_id = ?2)",
                params![target_context, id],
                |row| row.get(0),
            )?;
            if already_in_context {
                anyhow::bail!("\"{normalized_trigger}\" is already in this context");
            }
            if expansion != normalized_expansion || instructions != normalized_instructions {
                anyhow::bail!("\"{normalized_trigger}\" already exists with different content");
            }
            conn.execute(
                "INSERT OR IGNORE INTO snippet_contexts (context_id, snippet_id) VALUES (?1, ?2)",
                params![target_context, id],
            )?;
            return Ok(CreatedRecordMeta { id, created_at });
        }
    }

    conn.execute(
        "INSERT INTO snippets (trigger, expansion, instructions) VALUES (?1, ?2, ?3)",
        params![
            normalized_trigger,
            normalized_expansion,
            normalized_instructions
        ],
    )?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT OR IGNORE INTO snippet_contexts (context_id, snippet_id) VALUES (?1, ?2)",
        params![target_context.unwrap_or(everywhere_id), id],
    )?;
    let created_at = conn.query_row(
        "SELECT created_at FROM snippets WHERE id=?1",
        params![id],
        |r| r.get(0),
    )?;
    invalidate_snippet_cache();
    Ok(CreatedRecordMeta { id, created_at })
}

#[cfg(test)]
pub fn update_snippet(
    db: &Db,
    id: i64,
    trigger: &str,
    expansion: &str,
    instructions: &str,
) -> Result<()> {
    update_snippet_if_unchanged(db, id, trigger, expansion, instructions, None)
}

#[derive(serde::Deserialize)]
pub struct ExpectedSnippet {
    pub trigger: String,
    pub expansion: String,
    pub instructions: String,
}

#[derive(serde::Deserialize)]
pub struct SnippetMoveEdit {
    pub id: i64,
    pub expected: ExpectedSnippet,
    pub expansion: String,
    pub instructions: String,
}

pub fn update_snippet_if_unchanged(
    db: &Db, id: i64, trigger: &str, expansion: &str, instructions: &str,
    expected: Option<&ExpectedSnippet>,
) -> Result<()> {
    let mut conn = lock_conn(db)?;
    let conn = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if let Some(expected) = expected {
        let current: Option<(String, String, String)> = conn.query_row(
            "SELECT trigger, expansion, instructions FROM snippets WHERE id=?1", params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        if current != Some((expected.trigger.clone(), expected.expansion.clone(), expected.instructions.clone())) {
            anyhow::bail!("This snippet changed while you were editing. Close and reopen it before saving.");
        }
    }
    update_snippet_conn(&conn, id, trigger, expansion, instructions)?;
    conn.commit()?;
    invalidate_snippet_cache();
    Ok(())
}

pub(crate) fn update_snippet_conn(conn: &rusqlite::Connection, id: i64, trigger: &str, expansion: &str, instructions: &str) -> Result<()> {
    let (normalized_trigger, normalized_expansion, normalized_instructions) = normalize_snippet(trigger, expansion, instructions)?;
    let changed = conn.execute(
        "UPDATE snippets SET trigger=?2, expansion=?3, instructions=?4 WHERE id=?1",
        params![
            id,
            normalized_trigger,
            normalized_expansion,
            normalized_instructions
        ],
    )?;
    require_row_changed(changed, "Snippet", id)?;
    Ok(())
}

pub fn delete_snippet(db: &Db, id: i64) -> Result<()> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    delete_snippet_conn(&tx, id)?;
    tx.commit()?;
    invalidate_snippet_cache();
    Ok(())
}

pub(crate) fn delete_snippet_conn(tx: &rusqlite::Connection, id: i64) -> Result<()> {
    let changed = tx.execute("DELETE FROM snippets WHERE id=?1", params![id])?;
    require_row_changed(changed, "Snippet", id)?;
    tx.execute(
        "DELETE FROM snippet_contexts WHERE snippet_id = ?1",
        params![id],
    )?;
    Ok(())
}

pub fn query_snippets(db: &Db) -> Result<Vec<Snippet>> {
    let conn = lock_conn(db)?;
    let mut stmt = conn.prepare_cached(
        "SELECT id, trigger, expansion, instructions, use_count, created_at \
         FROM snippets ORDER BY created_at DESC",
    )?;
    let rows = stmt
        .query_map([], snippet_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
pub fn query_snippets_for_context(db: &Db, context_id: i64) -> Result<Vec<Snippet>> {
    let conn = lock_conn(db)?;
    query_snippets_for_context_conn(&conn, context_id)
}

fn query_snippets_for_context_conn(
    conn: &rusqlite::Connection,
    context_id: i64,
) -> Result<Vec<Snippet>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.trigger, s.expansion, s.instructions, s.use_count, s.created_at
         FROM snippets s
         INNER JOIN snippet_contexts sc ON sc.snippet_id = s.id
         WHERE sc.context_id = ?1
         ORDER BY s.created_at DESC",
    )?;
    let mut rows = stmt
        .query_map(params![context_id], snippet_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for row in &mut rows {
        row.assigned_to_context = true;
    }
    Ok(rows)
}

/// The effective snippet library is additive: Everywhere snippets are
/// available in every targeted group, followed by that group's own rows.
/// Triggers are globally unique, so a context cannot redefine an inherited
/// trigger with a different expansion.
pub fn query_effective_snippets_for_context(db: &Db, context_id: i64) -> Result<Vec<Snippet>> {
    let conn = lock_conn(db)?;
    let context = query_context_conn(&conn, context_id)?;
    let everywhere_id = ensure_everywhere_context_conn(&conn)?;
    let local = query_snippets_for_context_conn(&conn, context_id)?;
    if context.is_everywhere || context_id == everywhere_id {
        return Ok(local);
    }

    let everywhere = query_snippets_for_context_conn(&conn, everywhere_id)?;
    let everywhere_ids = everywhere
        .iter()
        .map(|snippet| snippet.id)
        .collect::<HashSet<_>>();
    let mut local = local;
    let local_ids = local
        .iter()
        .map(|snippet| snippet.id)
        .collect::<HashSet<_>>();
    for snippet in &mut local {
        snippet.inherited_from_everywhere = everywhere_ids.contains(&snippet.id);
        snippet.assigned_to_context = true;
    }
    let mut everywhere = everywhere;
    for snippet in &mut everywhere {
        snippet.inherited_from_everywhere = true;
        snippet.assigned_to_context = local_ids.contains(&snippet.id);
    }
    let mut effective = everywhere;
    let mut seen = effective
        .iter()
        .map(|snippet| snippet.id)
        .collect::<HashSet<_>>();
    effective.extend(local.into_iter().filter(|snippet| seen.insert(snippet.id)));
    Ok(effective)
}

/// Removes a snippet from Everywhere and assigns the existing row to a
/// specific context. The expansion, instructions, and use count remain on
/// the same global row.
pub fn move_snippet_entry_to_context(
    db: &Db,
    trigger: &str,
    target_context_id: i64,
    edit: Option<&SnippetMoveEdit>,
) -> Result<Snippet> {
    let normalized_trigger = require_nonempty_trimmed("Trigger", trigger)?;
    validate_char_limit("Trigger", &normalized_trigger, SNIPPET_TRIGGER_CHAR_LIMIT)?;

    let mut conn = lock_conn(db)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let everywhere_id = ensure_everywhere_context_conn(&tx)?;
    if target_context_id == everywhere_id {
        anyhow::bail!("The Everywhere context cannot be the move destination");
    }
    let target_is_everywhere: Option<bool> = tx
        .query_row(
            "SELECT is_everywhere FROM contexts WHERE id = ?1",
            params![target_context_id],
            |row| Ok(row.get::<_, i64>(0)? != 0),
        )
        .optional()?;
    match target_is_everywhere {
        Some(false) => {}
        Some(true) => anyhow::bail!("The Everywhere context cannot be the move destination"),
        None => anyhow::bail!("Context {target_context_id} was not found"),
    }

    let snippet_id: i64 = tx
        .query_row(
            "SELECT id FROM snippets WHERE trigger = ?1",
            params![normalized_trigger],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("\"{normalized_trigger}\" was not found"))?;
    if let Some(edit) = edit {
        let current: (String, String, String) = tx.query_row(
            "SELECT trigger, expansion, instructions FROM snippets WHERE id=?1", params![snippet_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if snippet_id != edit.id || current != (edit.expected.trigger.clone(), edit.expected.expansion.clone(), edit.expected.instructions.clone()) {
            anyhow::bail!("This snippet changed while you were editing. Close and reopen it before saving.");
        }
        let (_, expansion, instructions) = normalize_snippet(&normalized_trigger, &edit.expansion, &edit.instructions)?;
        tx.execute("UPDATE snippets SET expansion=?2, instructions=?3 WHERE id=?1", params![snippet_id, expansion, instructions])?;
    }
    let removed = tx.execute(
        "DELETE FROM snippet_contexts WHERE context_id = ?1 AND snippet_id = ?2",
        params![everywhere_id, snippet_id],
    )?;
    if removed == 0 {
        anyhow::bail!("\"{normalized_trigger}\" is not assigned to Everywhere");
    }
    tx.execute(
        "INSERT OR IGNORE INTO snippet_contexts (context_id, snippet_id)
         VALUES (?1, ?2)",
        params![target_context_id, snippet_id],
    )?;
    let snippet = tx.query_row(
        "SELECT id, trigger, expansion, instructions, use_count, created_at
         FROM snippets WHERE id = ?1",
        params![snippet_id],
        snippet_from_row,
    )?;
    tx.commit()?;
    invalidate_snippet_cache();
    Ok(snippet)
}

pub fn increment_snippet_use(db: &Db, id: i64) -> Result<()> {
    increment_snippet_use_counts(db, &[(id, 1)])
}

pub fn increment_snippet_use_counts(db: &Db, counts: &[(i64, i64)]) -> Result<()> {
    if counts.is_empty() {
        return Ok(());
    }
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("UPDATE snippets SET use_count = use_count + ?2 WHERE id=?1")?;
        for (id, count) in counts.iter().copied() {
            if count <= 0 {
                continue;
            }
            stmt.execute(params![id, count])?;
            super::library_usage::record_library_usage_conn(&tx, "snippet", id, count)?;
        }
    }
    tx.commit()?;
    Ok(())
}
