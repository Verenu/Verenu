#![allow(dead_code)]

//! Database schema definition, connection `open`, and versioned migrations.

use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use uuid::Uuid;

use super::*;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS transcriptions (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  raw_text    TEXT    NOT NULL,
  clean_text  TEXT    NOT NULL,
  words       INTEGER NOT NULL DEFAULT 0,
  spoken_words INTEGER,
  duration_ms INTEGER NOT NULL DEFAULT 0,
  api_used    TEXT    NOT NULL DEFAULT '',
  context_id  INTEGER,
  created_at  DATETIME NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_transcriptions_created_at
  ON transcriptions(created_at);
CREATE TABLE IF NOT EXISTS lifetime_stats (
  id               INTEGER PRIMARY KEY CHECK (id = 1),
  total_words      INTEGER NOT NULL DEFAULT 0,
  dictionary_fixes INTEGER NOT NULL DEFAULT 0,
  wpm_sum          REAL NOT NULL DEFAULT 0,
  wpm_count        INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS transcription_daily_stats (
  day                TEXT PRIMARY KEY,
  total_words        INTEGER NOT NULL DEFAULT 0,
  total_transcriptions INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS dictionary (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  term             TEXT    NOT NULL UNIQUE,
  mistake          TEXT,
  auto_learned     INTEGER NOT NULL DEFAULT 0,
  correction_count INTEGER NOT NULL DEFAULT 0,
  confidence_tier  TEXT    NOT NULL DEFAULT 'low',
  last_seen_at     DATETIME,
  created_at       DATETIME NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS snippets (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  trigger      TEXT    NOT NULL UNIQUE,
  expansion    TEXT    NOT NULL,
  instructions TEXT    NOT NULL DEFAULT '',
  use_count    INTEGER NOT NULL DEFAULT 0,
  created_at   DATETIME NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS contexts (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  name              TEXT NOT NULL COLLATE NOCASE UNIQUE,
  is_everywhere     INTEGER NOT NULL DEFAULT 0 CHECK (is_everywhere IN (0, 1)),
  icon              TEXT,
  tone              TEXT,
  cleanup_intensity TEXT,
  color             TEXT,
  custom_instructions TEXT,
  contextual_formatting_disabled INTEGER NOT NULL DEFAULT 0 CHECK (contextual_formatting_disabled IN (0, 1)),
  pinned_at         DATETIME,
  created_at        DATETIME NOT NULL DEFAULT (datetime('now')),
  updated_at        DATETIME NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_contexts_everywhere
  ON contexts(is_everywhere) WHERE is_everywhere = 1;
CREATE TABLE IF NOT EXISTS context_targets (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  context_id   INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
  executable   TEXT NOT NULL COLLATE NOCASE UNIQUE,
  app_name     TEXT,
  developer    TEXT,
  platform     TEXT,
  created_at   DATETIME NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_context_targets_context_id
  ON context_targets(context_id);
CREATE TABLE IF NOT EXISTS context_website_targets (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  context_id   INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
  domain       TEXT NOT NULL COLLATE NOCASE UNIQUE,
  created_at   DATETIME NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_context_website_targets_context_id
  ON context_website_targets(context_id);
CREATE TABLE IF NOT EXISTS dictionary_contexts (
  context_id    INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
  dictionary_id INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
  PRIMARY KEY (context_id, dictionary_id)
);
CREATE INDEX IF NOT EXISTS idx_dictionary_contexts_dictionary_id
  ON dictionary_contexts(dictionary_id);
-- A canonical dictionary term may be shared by many contexts, but the
-- mistranscription that produces it is context-specific.  Keep one row per
-- effective variant so a rejection can identify exactly the mapping that
-- fired instead of deleting the shared canonical row.
CREATE TABLE IF NOT EXISTS dictionary_corrections (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  uuid             TEXT,
  context_id       INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
  dictionary_id    INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
  mistake          TEXT NOT NULL COLLATE NOCASE,
  auto_learned     INTEGER NOT NULL DEFAULT 0 CHECK (auto_learned IN (0, 1)),
  correction_count INTEGER NOT NULL DEFAULT 0,
  confidence_tier  TEXT NOT NULL DEFAULT 'low',
  last_seen_at     DATETIME,
  created_at       DATETIME NOT NULL DEFAULT (datetime('now')),
  UNIQUE(context_id, dictionary_id, mistake)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_dictionary_corrections_uuid
  ON dictionary_corrections(uuid) WHERE uuid IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_dictionary_corrections_context_mistake
  ON dictionary_corrections(context_id, mistake);
CREATE INDEX IF NOT EXISTS idx_dictionary_corrections_context_dictionary
  ON dictionary_corrections(context_id, dictionary_id);
CREATE TRIGGER IF NOT EXISTS trg_dictionary_contexts_delete_corrections
AFTER DELETE ON dictionary_contexts BEGIN
  DELETE FROM dictionary_corrections
   WHERE context_id = OLD.context_id AND dictionary_id = OLD.dictionary_id;
END;
CREATE TABLE IF NOT EXISTS snippet_contexts (
  context_id INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
  snippet_id INTEGER NOT NULL REFERENCES snippets(id) ON DELETE CASCADE,
  PRIMARY KEY (context_id, snippet_id)
);
CREATE INDEX IF NOT EXISTS idx_snippet_contexts_snippet_id
  ON snippet_contexts(snippet_id);
CREATE TABLE IF NOT EXISTS pending_corrections (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  context_id   INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
  wrong_word   TEXT    NOT NULL,
  correct_word TEXT    NOT NULL,
  created_at   DATETIME NOT NULL DEFAULT (datetime('now'))
);
-- This index is created after the v26 shape migration. Do not create it here:
-- SCHEMA runs before migrations and an existing pre-v26 table has no
-- context_id column yet.
CREATE TABLE IF NOT EXISTS auto_learn_events (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  event_type     TEXT    NOT NULL,
  reason_code    TEXT    NOT NULL DEFAULT '',
  context_id     INTEGER REFERENCES contexts(id) ON DELETE SET NULL,
  app_context    TEXT    NOT NULL DEFAULT '',
  mistake_hash   TEXT    NOT NULL DEFAULT '',
  correction_hash TEXT   NOT NULL DEFAULT '',
  confidence     REAL    NOT NULL DEFAULT 0.0,
  created_at     DATETIME NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_auto_learn_events_event_type
  ON auto_learn_events(event_type, created_at);
CREATE TABLE IF NOT EXISTS auto_learn_candidates (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  context_id         INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
  wrong_word         TEXT    NOT NULL,
  correct_word       TEXT    NOT NULL,
  confidence_sum     REAL    NOT NULL DEFAULT 0.0,
  confidence_avg     REAL    NOT NULL DEFAULT 0.0,
  seen_count         INTEGER NOT NULL DEFAULT 0,
  last_seen_at       DATETIME NOT NULL DEFAULT (datetime('now')),
  cooldown_until     DATETIME,
  promoted_at        DATETIME,
  UNIQUE(context_id, wrong_word, correct_word)
);
-- This index is created after the v26 shape migration. Do not create it here:
-- SCHEMA runs before migrations and an existing pre-v26 table has no
-- context_id column yet.
CREATE TABLE IF NOT EXISTS cleanup_cache (
  key         TEXT PRIMARY KEY,
  clean_text  TEXT NOT NULL,
  hit_count   INTEGER NOT NULL DEFAULT 0,
  created_at  DATETIME NOT NULL DEFAULT (datetime('now')),
  last_hit_at DATETIME NOT NULL DEFAULT (datetime('now')),
  expires_at  DATETIME NOT NULL,
  created_at_epoch  INTEGER,
  last_hit_at_epoch INTEGER,
  expires_at_epoch  INTEGER,
  is_snippet  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_cleanup_cache_expires_at
  ON cleanup_cache(expires_at);
CREATE INDEX IF NOT EXISTS idx_cleanup_cache_last_hit_at
  ON cleanup_cache(last_hit_at);
CREATE TABLE IF NOT EXISTS seeded_defaults (
  key TEXT PRIMARY KEY
);
CREATE TABLE IF NOT EXISTS api_calls (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  transcription_id  INTEGER NOT NULL,
  model             TEXT    NOT NULL,
  provider          TEXT    NOT NULL,
  task              TEXT    NOT NULL,
  audio_ms          INTEGER NOT NULL DEFAULT 0,
  input_chars       INTEGER NOT NULL DEFAULT 0,
  output_chars      INTEGER NOT NULL DEFAULT 0,
  created_at        DATETIME NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_api_calls_created_at
  ON api_calls(created_at);
CREATE INDEX IF NOT EXISTS idx_api_calls_transcription_id
  ON api_calls(transcription_id);
CREATE TABLE IF NOT EXISTS openrouter_pricing (
  model_id                TEXT PRIMARY KEY COLLATE NOCASE,
  prompt_usd_per_token    REAL NOT NULL,
  completion_usd_per_token REAL NOT NULL,
  fetched_at              INTEGER NOT NULL
);
";

/// Opens the database, and if it (or its WAL sidecar) is corrupt, quarantines
/// the corrupt files and retries with a fresh database instead of failing the
/// whole app. Used at startup: a `db::open` panic there would otherwise put
/// Verenu in a crash loop (the startup-recovery relaunch re-panics on the
/// same corrupt file), with no way for the user to recover besides manually
/// deleting their database. Only a truly unwritable directory (fresh open also
/// failing) is allowed to bubble up as a hard startup error.
pub fn open_with_recovery(path: impl AsRef<std::path::Path>) -> Result<Db> {
    match open(path.as_ref()) {
        Ok(db) => Ok(db),
        Err(first_err) => {
            // Quarantine only genuine corruption. A transient failure (file
            // locked or busy, an unwritable directory, an I/O error) is not
            // corruption: moving a healthy database aside would silently
            // discard the user's history in favor of a fresh empty file.
            if !open_error_is_corruption(&first_err) || !quarantine_corrupt_db_files(path.as_ref())
            {
                return Err(first_err);
            }
            log::error!(
                "database failed to open and was moved aside for diagnosis; starting with a fresh database: {first_err}"
            );
            open(path.as_ref()).map_err(|second_err| {
                anyhow::anyhow!(
                    "database failed to open ({first_err}); quarantined and fresh reopen also failed: {second_err}"
                )
            })
        }
    }
}

/// Whether a failed [`open`] is caused by an actually corrupt database rather
/// than a transient or environmental error. `SQLITE_NOTADB` covers a main file
/// that is not a SQLite database and a wedged WAL; `SQLITE_CORRUPT` covers a
/// malformed database image or schema. Everything else (busy/locked, cannot
/// open, I/O) is left untouched so the user's data is never moved aside for a
/// problem that is not corruption.
fn open_error_is_corruption(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(code, _))
            if code.code == rusqlite::ErrorCode::NotADatabase
                || code.code == rusqlite::ErrorCode::DatabaseCorrupt
    )
}

/// Renames the database file plus its WAL/SHM sidecars to
/// `.corrupt-<nanos>` names so they are preserved for diagnosis but can never
/// block a fresh open. Returns `false` (and leaves the files in place) if any
/// rename fails, so the caller keeps the original error instead of masking it
/// with a worse one. A database whose WAL file is wedged is just as
/// un-openable as a bad main file, so all three are quarantined together.
pub(crate) fn quarantine_corrupt_db_files(db_path: &std::path::Path) -> bool {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut all_moved = true;
    for extension in ["db", "db-wal", "db-shm"] {
        let mut candidate = db_path.to_path_buf();
        candidate.set_extension(extension);
        if !candidate.exists() {
            continue;
        }
        let mut quarantined = candidate.clone();
        quarantined.set_extension(format!("{extension}.corrupt-{nanos}"));
        if let Err(err) = std::fs::rename(&candidate, &quarantined) {
            // File name only — the full path is a user-local file location
            // that must not appear in logs.
            log::warn!(
                "failed to quarantine corrupt database file {:?}: {err}",
                candidate.file_name()
            );
            all_moved = false;
        }
    }
    all_moved
}

pub fn open(path: impl AsRef<std::path::Path>) -> Result<Db> {
    let db_path = path.as_ref();
    // Connection::open creates the file if it doesn't exist, so this check
    // must run before open() - otherwise it's always true (even on a brand
    // new install) and a pointless db.bak gets created on first launch.
    let db_existed_before_open = db_path.exists();
    let mut conn = Connection::open(db_path)?;
    // SQLite keeps foreign-key enforcement disabled per connection by
    // default. Context-owned correction/evidence rows use cascading foreign
    // keys, so enable enforcement before schema work or application queries.
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    // user_version lives in the file header, so it's readable before SCHEMA
    // is applied. Read it here (and take the pre-migration backup) before any
    // statement touches the file, so db.bak is a true snapshot of what the
    // user had - not a copy already overwritten by SCHEMA's CREATE TABLE IF
    // NOT EXISTS statements.
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if user_version < 2 && db_existed_before_open {
        let _ = std::fs::copy(db_path, db_path.with_extension("db.bak"));
    }

    conn.execute_batch(SCHEMA)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;

    if user_version < 2 {
        // IMPORTANT: each migration uses BEGIN/COMMIT for atomicity, followed by an
        // explicit ROLLBACK. If the migration fails mid-way, sqlite3_exec aborts but
        // leaves the BEGIN open. Without the ROLLBACK cleanup, every subsequent INSERT
        // would execute inside that ghost transaction and be silently discarded on
        // connection close — causing all user data to vanish on restart.
        let _ = conn.execute_batch(
            "ALTER TABLE snippets ADD COLUMN instructions TEXT NOT NULL DEFAULT '';",
        );
        let _ = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_corrections (
               id           INTEGER PRIMARY KEY AUTOINCREMENT,
               wrong_word   TEXT    NOT NULL,
               correct_word TEXT    NOT NULL,
               created_at   DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             CREATE INDEX IF NOT EXISTS idx_pending_words
               ON pending_corrections(wrong_word, correct_word);",
        );
        // Migrate dictionary to final schema: term (required) + mistake (optional).
        // Handles all prior states (original wrong/correct columns, or already migrated).
        let _ = conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS dictionary_v3 (
                 id               INTEGER PRIMARY KEY AUTOINCREMENT,
                 term             TEXT    NOT NULL UNIQUE,
                 mistake          TEXT,
                 auto_learned     INTEGER NOT NULL DEFAULT 0,
                 correction_count INTEGER NOT NULL DEFAULT 0,
                 created_at       DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             INSERT OR IGNORE INTO dictionary_v3
                 (id, term, mistake, auto_learned, correction_count, created_at)
                 SELECT id,
                        COALESCE(correct, wrong),
                        CASE WHEN correct IS NOT NULL THEN wrong ELSE NULL END,
                        auto_learned, correction_count, created_at
                 FROM dictionary;
             DROP TABLE dictionary;
             ALTER TABLE dictionary_v3 RENAME TO dictionary;
             COMMIT;",
        );
        // Clean up any dangling transaction left by a failed migration above.
        let _ = conn.execute_batch("ROLLBACK;");
        conn.execute_batch("PRAGMA user_version = 2;")?;
    }
    if user_version == 2 {
        // Older v2 databases can have the legacy dictionary shape and a
        // snippets table that predates the instructions column. Re-run the
        // idempotent v2 repair before advancing through later migrations.
        run_migration(&mut conn, apply_v2_migration)?;
    }
    if user_version < 3 {
        let res = conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS cleanup_cache (
               key         TEXT PRIMARY KEY,
               clean_text  TEXT NOT NULL,
               hit_count   INTEGER NOT NULL DEFAULT 0,
               created_at  DATETIME NOT NULL DEFAULT (datetime('now')),
               last_hit_at DATETIME NOT NULL DEFAULT (datetime('now')),
               expires_at  DATETIME NOT NULL,
               created_at_epoch  INTEGER,
               last_hit_at_epoch INTEGER,
               expires_at_epoch  INTEGER
              );
             CREATE INDEX IF NOT EXISTS idx_cleanup_cache_expires_at
               ON cleanup_cache(expires_at);
             CREATE INDEX IF NOT EXISTS idx_cleanup_cache_last_hit_at
               ON cleanup_cache(last_hit_at);
             PRAGMA user_version = 3;
             COMMIT;",
        );
        if let Err(err) = res {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(err.into());
        }
    }
    if user_version < 4 {
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "dictionary",
                "confidence_tier",
                "ALTER TABLE dictionary ADD COLUMN confidence_tier TEXT NOT NULL DEFAULT 'low';",
            )?;
            ensure_table_column(
                conn,
                "dictionary",
                "last_seen_at",
                "ALTER TABLE dictionary ADD COLUMN last_seen_at DATETIME;",
            )?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS auto_learn_events (
                   id             INTEGER PRIMARY KEY AUTOINCREMENT,
                   event_type     TEXT    NOT NULL,
                   reason_code    TEXT    NOT NULL DEFAULT '',
                   app_context    TEXT    NOT NULL DEFAULT '',
                   mistake_hash   TEXT    NOT NULL DEFAULT '',
                   correction_hash TEXT   NOT NULL DEFAULT '',
                   confidence     REAL    NOT NULL DEFAULT 0.0,
                   created_at     DATETIME NOT NULL DEFAULT (datetime('now'))
                 );
                 CREATE INDEX IF NOT EXISTS idx_auto_learn_events_event_type
                   ON auto_learn_events(event_type, created_at);
                 CREATE TABLE IF NOT EXISTS auto_learn_candidates (
                   id                 INTEGER PRIMARY KEY AUTOINCREMENT,
                   wrong_word         TEXT    NOT NULL,
                   correct_word       TEXT    NOT NULL,
                   confidence_sum     REAL    NOT NULL DEFAULT 0.0,
                   confidence_avg     REAL    NOT NULL DEFAULT 0.0,
                   seen_count         INTEGER NOT NULL DEFAULT 0,
                   last_seen_at       DATETIME NOT NULL DEFAULT (datetime('now')),
                   cooldown_until     DATETIME,
                   promoted_at        DATETIME,
                   UNIQUE(wrong_word, correct_word)
                 );
                 CREATE INDEX IF NOT EXISTS idx_auto_learn_candidates_seen
                   ON auto_learn_candidates(last_seen_at);
                 PRAGMA user_version = 4;",
            )?;
            Ok(())
        })?;
    }
    if user_version < 5 {
        run_migration(&mut conn, |conn| {
            ensure_cleanup_cache_schema(conn)?;
            conn.execute_batch("PRAGMA user_version = 5;")?;
            Ok(())
        })?;
    }
    if user_version < 6 {
        run_migration(&mut conn, |conn| {
            ensure_cleanup_cache_schema(conn)?;
            conn.execute_batch("PRAGMA user_version = 6;")?;
            Ok(())
        })?;
    }
    if user_version < 7 {
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "transcriptions",
                "spoken_words",
                "ALTER TABLE transcriptions ADD COLUMN spoken_words INTEGER;",
            )?;
            backfill_spoken_words(conn)?;
            conn.execute_batch("PRAGMA user_version = 7;")?;
            Ok(())
        })?;
    }
    if user_version < 8 {
        let res = conn.execute_batch(
            "BEGIN;
             CREATE INDEX IF NOT EXISTS idx_transcriptions_created_at
               ON transcriptions(created_at);
             PRAGMA user_version = 8;
             COMMIT;",
        );
        if let Err(err) = res {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(err.into());
        }
    }
    if user_version < 9 {
        // Seed the lifetime word counter from existing history so upgrading
        // users don't see it reset to zero. From here on it's only ever
        // incremented on insert, never recomputed from transcriptions, so
        // history retention pruning can't shrink it.
        let res = conn.execute_batch(
            "BEGIN;
             INSERT OR IGNORE INTO lifetime_stats (id, total_words)
               SELECT 1, COALESCE(SUM(words), 0) FROM transcriptions;
             PRAGMA user_version = 9;
             COMMIT;",
        );
        if let Err(err) = res {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(err.into());
        }
    }
    if user_version < 10 {
        // Per-call API usage records for the Insights cost card. Written
        // from the pipeline at finalize time; historical transcriptions
        // predating this table simply have no cost data. Also declared in
        // SCHEMA above so an interrupted migration can't leave a database
        // without the table (the CREATE TABLE IF NOT EXISTS self-heals).
        let res = conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS api_calls (
               id                INTEGER PRIMARY KEY AUTOINCREMENT,
               transcription_id  INTEGER NOT NULL,
               model             TEXT    NOT NULL,
               provider          TEXT    NOT NULL,
               task              TEXT    NOT NULL,
               audio_ms          INTEGER NOT NULL DEFAULT 0,
               input_chars       INTEGER NOT NULL DEFAULT 0,
               output_chars      INTEGER NOT NULL DEFAULT 0,
               created_at        DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             CREATE INDEX IF NOT EXISTS idx_api_calls_created_at
               ON api_calls(created_at);
             PRAGMA user_version = 10;
             COMMIT;",
        );
        if let Err(err) = res {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(err.into());
        }
    }
    if user_version < 11 {
        // Real lifetime counter for dictionary substitutions actually
        // applied to dictations (incremented from the pipeline with the
        // `applied_dict_ids` count). Mirrors `total_words`: never recomputed
        // from history, so retention pruning can't shrink it. The column is
        // declared in SCHEMA for fresh databases; ensure_table_column is
        // idempotent for databases that already have it.
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "lifetime_stats",
                "dictionary_fixes",
                "ALTER TABLE lifetime_stats ADD COLUMN dictionary_fixes INTEGER NOT NULL DEFAULT 0;",
            )?;
            conn.execute_batch("PRAGMA user_version = 11;")?;
            Ok(())
        })?;
    }
    if user_version < 12 {
        log::info!("db: migrating schema {user_version} -> 12");
        run_migration(&mut conn, |conn| {
            apply_v12_context_migration(conn)?;
            conn.execute_batch("PRAGMA user_version = 12;")?;
            Ok(())
        })?;
    }
    if user_version < 13 {
        log::info!("db: migrating schema {user_version} -> 13");
        // Per-dictation foreground app (the lowercase executable name, e.g.
        // "outlook.exe") so History can filter/annotate by app. The value was
        // never persisted before v13, so past rows keep NULL and simply have
        // no app metadata. Declared in SCHEMA for fresh databases;
        // ensure_table_column is idempotent for databases that already have it.
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "transcriptions",
                "app_name",
                "ALTER TABLE transcriptions ADD COLUMN app_name TEXT;",
            )?;
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_transcriptions_app_name
                   ON transcriptions(app_name);
                 PRAGMA user_version = 13;",
            )?;
            Ok(())
        })?;
    }
    if user_version < 14 {
        log::info!("db: migrating schema {user_version} -> 14");
        // Per-context icon/tone/cleanup override and website-domain targets,
        // so a context can be activated by browser domain (not just exe) and
        // can override the global tone/cleanup intensity while active.
        // Columns declared in SCHEMA for fresh databases; ensure_table_column
        // is idempotent for databases that already have them.
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "contexts",
                "icon",
                "ALTER TABLE contexts ADD COLUMN icon TEXT;",
            )?;
            ensure_table_column(
                conn,
                "contexts",
                "tone",
                "ALTER TABLE contexts ADD COLUMN tone TEXT;",
            )?;
            ensure_table_column(
                conn,
                "contexts",
                "cleanup_intensity",
                "ALTER TABLE contexts ADD COLUMN cleanup_intensity TEXT;",
            )?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS context_website_targets (
                   id           INTEGER PRIMARY KEY AUTOINCREMENT,
                   context_id   INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
                   domain       TEXT NOT NULL COLLATE NOCASE UNIQUE,
                   created_at   DATETIME NOT NULL DEFAULT (datetime('now'))
                 );
                 CREATE INDEX IF NOT EXISTS idx_context_website_targets_context_id
                   ON context_website_targets(context_id);
                 PRAGMA user_version = 14;",
            )?;
            Ok(())
        })?;
    }
    if user_version < 15 {
        log::info!("db: migrating schema {user_version} -> 15");
        // Per-context accent color (a small curated swatch, picked via
        // right-click on the context's sidebar icon) so contexts can be
        // told apart at a glance in the rail. Column declared in SCHEMA for
        // fresh databases; ensure_table_column is idempotent for databases
        // that already have it.
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "contexts",
                "color",
                "ALTER TABLE contexts ADD COLUMN color TEXT;",
            )?;
            conn.execute_batch("PRAGMA user_version = 15;")?;
            Ok(())
        })?;
    }
    if user_version < 16 {
        log::info!("db: migrating schema {user_version} -> 16");
        // Per-context free-text instructions sent directly to the cleanup LLM
        // alongside the tone/cleanup overrides. Column declared in SCHEMA for
        // fresh databases; ensure_table_column is idempotent for databases
        // that already have it.
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "contexts",
                "custom_instructions",
                "ALTER TABLE contexts ADD COLUMN custom_instructions TEXT;",
            )?;
            conn.execute_batch("PRAGMA user_version = 16;")?;
            Ok(())
        })?;
    }
    if user_version < 17 {
        log::info!("db: migrating schema {user_version} -> 17");
        // Pin timestamp for the sidebar's context list: NULL means unpinned,
        // and pinned rows sort newest-pin-first above the creation-ordered
        // rest. Column declared in SCHEMA for fresh databases;
        // ensure_table_column is idempotent for databases that already have it.
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "contexts",
                "pinned_at",
                "ALTER TABLE contexts ADD COLUMN pinned_at DATETIME;",
            )?;
            conn.execute_batch("PRAGMA user_version = 17;")?;
            Ok(())
        })?;
    }
    if user_version < 18 {
        log::info!("db: migrating schema {user_version} -> 18");
        // Which context a dictation ran under, so the context page and the
        // Insights filter can show real per-context totals. Deliberately not a
        // foreign key: deleting a context must not delete its history, and a
        // stale id simply stops matching. Rows dictated before v18 keep NULL
        // and count toward nothing. Column declared in SCHEMA for fresh
        // databases; ensure_table_column is idempotent for databases that
        // already have it.
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "transcriptions",
                "context_id",
                "ALTER TABLE transcriptions ADD COLUMN context_id INTEGER;",
            )?;
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_transcriptions_context_id
                   ON transcriptions(context_id);
                 PRAGMA user_version = 18;",
            )?;
            Ok(())
        })?;
    }
    if user_version < 19 {
        log::info!("db: migrating schema {user_version} -> 19");
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "contexts",
                "contextual_formatting_disabled",
                "ALTER TABLE contexts ADD COLUMN contextual_formatting_disabled INTEGER NOT NULL DEFAULT 0 CHECK (contextual_formatting_disabled IN (0, 1));",
            )?;
            conn.execute_batch("PRAGMA user_version = 19;")?;
            Ok(())
        })?;
    }
    if user_version < 20 {
        log::info!("db: migrating schema {user_version} -> 20");
        run_migration(&mut conn, |conn| {
            apply_v20_sync_migration(conn)?;
            conn.execute_batch("PRAGMA user_version = 20;")?;
            Ok(())
        })?;
    }
    if user_version < 21 {
        log::info!("db: migrating schema {user_version} -> 21");
        // Which OS an exe target was assigned on. NULL (all pre-v21 rows) means
        // "unknown platform" and is treated as visible everywhere — sync keeps
        // syncing the raw executable string across devices regardless, but a
        // tagged row is only shown/offered for removal on the device whose OS
        // produced that executable naming convention.
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "context_targets",
                "platform",
                "ALTER TABLE context_targets ADD COLUMN platform TEXT;",
            )?;
            conn.execute_batch("PRAGMA user_version = 21;")?;
            Ok(())
        })?;
    }
    if user_version < 22 {
        log::info!("db: migrating schema {user_version} -> 22");
        // History retention is device-local. Older sync builds logged every
        // retention prune as a transcription delete, which let one device's
        // shorter retention window erase history on its peers. Existing
        // tombstones are retention deletes (there is no user-facing history
        // delete action), so discard them and stop creating new ones.
        run_migration(&mut conn, |conn| {
            conn.execute_batch(
                "DROP TRIGGER IF EXISTS trg_sync_transcriptions_del;
                 DELETE FROM sync_log
                  WHERE table_name = 'transcriptions' AND op = 'delete';
                 PRAGMA user_version = 22;",
            )?;
            Ok(())
        })?;
    }
    if user_version < 23 {
        log::info!("db: migrating schema {user_version} -> 23");
        run_migration(&mut conn, |conn| {
            ensure_table_column(
                conn,
                "context_targets",
                "app_name",
                "ALTER TABLE context_targets ADD COLUMN app_name TEXT;",
            )?;
            ensure_table_column(
                conn,
                "context_targets",
                "developer",
                "ALTER TABLE context_targets ADD COLUMN developer TEXT;",
            )?;
            conn.execute_batch("PRAGMA user_version = 23;")?;
            Ok(())
        })?;
    }
    if user_version < 24 {
        log::info!("db: migrating schema {user_version} -> 24");
        run_migration(&mut conn, |conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS openrouter_pricing (
                   model_id                 TEXT PRIMARY KEY COLLATE NOCASE,
                   prompt_usd_per_token     REAL NOT NULL,
                   completion_usd_per_token REAL NOT NULL,
                   fetched_at               INTEGER NOT NULL
                 );
                 PRAGMA user_version = 24;",
            )?;
            Ok(())
        })?;
    }
    if user_version < 25 {
        log::info!("db: migrating schema {user_version} -> 25");
        run_migration(&mut conn, |conn| {
            let added_spoken_words = ensure_table_column(
                conn,
                "transcriptions",
                "spoken_words",
                "ALTER TABLE transcriptions ADD COLUMN spoken_words INTEGER;",
            )?;
            if added_spoken_words {
                backfill_spoken_words(conn)?;
            }
            ensure_table_column(
                conn,
                "lifetime_stats",
                "wpm_sum",
                "ALTER TABLE lifetime_stats ADD COLUMN wpm_sum REAL NOT NULL DEFAULT 0;",
            )?;
            ensure_table_column(
                conn,
                "lifetime_stats",
                "wpm_count",
                "ALTER TABLE lifetime_stats ADD COLUMN wpm_count INTEGER NOT NULL DEFAULT 0;",
            )?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS transcription_daily_stats (
                   day                  TEXT PRIMARY KEY,
                   total_words          INTEGER NOT NULL DEFAULT 0,
                   total_transcriptions INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT OR IGNORE INTO lifetime_stats (id, total_words)
                   VALUES (1, 0);
                 UPDATE lifetime_stats
                    SET wpm_sum = COALESCE((
                          SELECT SUM(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words, words) > 0
                                          THEN CAST(COALESCE(spoken_words, words) AS REAL) * 60000.0 / duration_ms
                                          ELSE 0 END)
                          FROM transcriptions
                        ), 0),
                        wpm_count = COALESCE((
                          SELECT COUNT(*) FROM transcriptions
                           WHERE duration_ms > 0 AND COALESCE(spoken_words, words) > 0
                        ), 0)
                  WHERE id = 1;
                 INSERT OR REPLACE INTO transcription_daily_stats (day, total_words, total_transcriptions)
                   SELECT date(created_at, 'localtime'), COALESCE(SUM(words), 0), COUNT(*)
                     FROM transcriptions
                    GROUP BY date(created_at, 'localtime');
                 PRAGMA user_version = 25;",
            )?;
            Ok(())
        })?;
    }
    if user_version < 26 {
        log::info!("db: migrating schema {user_version} -> 26");
        run_migration(&mut conn, |conn| {
            apply_v26_autolearn_context_migration(conn)?;
            conn.execute_batch("PRAGMA user_version = 26;")?;
            Ok(())
        })?;
    }
    // SCHEMA executes before migrations so it can safely create missing
    // tables, but it cannot create a context-aware index against a legacy
    // pre-v26 table. The v26 rebuild above installs these indexes for an
    // upgrade, and this idempotent check keeps fresh and already-v26 files
    // equally healthy on every reopen.
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_pending_words
           ON pending_corrections(context_id, wrong_word, correct_word, created_at);
         CREATE INDEX IF NOT EXISTS idx_auto_learn_candidates_seen
           ON auto_learn_candidates(context_id, last_seen_at);",
    )?;
    ensure_dictionary_correction_membership_trigger(&conn)?;
    // Early v20 development databases created sync_peers before receive and
    // send cursors were split. Their version marker is already 20, so the
    // migration above will not run again. Repair that partial v20 shape on
    // every open before pairing can write or query the missing cursor.
    ensure_table_column(
        &conn,
        "sync_peers",
        "recv_cursor",
        "ALTER TABLE sync_peers ADD COLUMN recv_cursor INTEGER NOT NULL DEFAULT 0;",
    )?;
    // Triggers can run before the async sync manager finishes loading the
    // keychain identity. Keep a provisional UUID in place so early writes are
    // captured; initialize() replaces it with the durable identity UUID.
    ensure_sync_identity_placeholder(&conn)?;
    // A partially completed v26 install may have created correction rows but
    // not reached the trigger bundle before its process stopped. Repair their
    // stable identities before re-installing the idempotent trigger set. The
    // UUID backfill intentionally precedes the triggers so repair itself does
    // not create a user-visible sync edit.
    backfill_canonical_uuids(&conn, "dictionary_corrections")?;
    conn.execute_batch(SYNC_TRIGGER_SQL)?;
    ensure_cleanup_cache_schema(&conn)?;
    // Index only needed by existing databases: the SCHEMA above declares it
    // for fresh installs, and the v10 migration block adds it for databases
    // created between v10 and v11. This line heals any database that predates
    // the column's index.
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_api_calls_transcription_id
           ON api_calls(transcription_id);",
    )?;

    // Self-heal: some databases ended up with user_version >= 7 without the
    // spoken_words column from that migration actually landing (an
    // interrupted migration during the Verenu rename/update). The column
    // addition and backfill run in one transaction so a failed backfill
    // rolls back the column too, letting this retry on the next launch.
    // Errors are propagated: without this column, every transcription
    // insert fails, so running in this state is worse than failing to open.
    if !table_has_column(&conn, "transcriptions", "spoken_words")? {
        let tx = conn.transaction()?;
        let res = (|| -> Result<()> {
            tx.execute_batch("ALTER TABLE transcriptions ADD COLUMN spoken_words INTEGER;")?;
            backfill_spoken_words(&tx)?;
            Ok(())
        })();
        match res {
            Ok(()) => {
                tx.commit()?;
            }
            Err(err) => {
                log::error!("Failed to self-heal spoken_words column: {err}");
                return Err(err);
            }
        }
    }

    ensure_stats_summary_triggers(&conn)?;
    ensure_history_fts(&conn);

    Ok(Arc::new(Mutex::new(conn)))
}

/// Reclaim SQLite sidecar/free-page space. Existing databases often predate
/// incremental auto-vacuum, so incremental_vacuum alone is a no-op for them.
/// Only incremental auto-vacuum runs here. A full VACUUM must use an explicit
/// user-idle maintenance flow because it holds the shared connection.
pub fn sqlite_disk_maintenance(db: &Db) -> Result<()> {
    let conn = lock_conn(db)?;
    let _: (i64, i64, i64) = conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |row| row.get(0))?;
    let page_count: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    let freelist_count: i64 = conn.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
    let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    // Reclaim in small chunks only when the free list is both material and a
    // meaningful fraction of the file. A full VACUUM needs an explicit,
    // user-idle maintenance flow: holding this shared connection while SQLite
    // rewrites the database would pause recording, history, and sync work.
    if freelist_count < 256 || freelist_count.saturating_mul(4) < page_count || page_size <= 0 {
        return Ok(());
    }
    if auto_vacuum == 2 {
        let pages = freelist_count.min(512);
        conn.execute(&format!("PRAGMA incremental_vacuum({pages})"), [])?;
    }
    Ok(())
}

/// Adds the LAN device-sync layer (v20) without changing any existing row
/// shapes:
///
/// - Every syncable table gains a `uuid` column (backfilled with canonical
///   hyphenated UUIDs)
///   plus a unique index, so records have stable identities that mean the same
///   thing on every paired device. Local integer ids stay the primary keys.
/// - `sync_log` records one row per captured mutation (table, row uuid, op,
///   timestamp, originating device). Peers pull deltas by log position, so
///   content deletes propagate exactly and no tombstone rows pollute the main
///   tables. Transcription history is append-only for sync: retention cleanup
///   is local to each device and never becomes a delete op.
/// - AFTER triggers on the syncable tables append to `sync_log`. The triggers
///   stay silent while `sync_state.applying` is 1 - that flag is how the sync
///   engine applies a remote change and records it in the log once (with the
///   remote's original timestamp/origin) instead of echoing it as a local edit.
/// - `sync_peers` holds pairing/trust state, `sync_remote_stats` holds other
///   devices' lifetime counters, `sync_setting_meta` holds per-key sync
///   timestamps for settings.json, and `sync_identity` mirrors this device's
///   uuid so trigger bodies can stamp the origin.
fn apply_v20_sync_migration(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_state (
           applying INTEGER NOT NULL DEFAULT 0
         );
         DELETE FROM sync_state
          WHERE rowid NOT IN (SELECT rowid FROM sync_state LIMIT 1);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_sync_state_singleton
           ON sync_state ((1));
         INSERT OR IGNORE INTO sync_state (applying) VALUES (0);
         CREATE TABLE IF NOT EXISTS sync_identity (
           uuid TEXT NOT NULL,
           name TEXT NOT NULL DEFAULT ''
         );
         DELETE FROM sync_identity
          WHERE rowid NOT IN (SELECT rowid FROM sync_identity LIMIT 1);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_sync_identity_singleton
           ON sync_identity ((1));
         CREATE TABLE IF NOT EXISTS sync_log (
           seq        INTEGER PRIMARY KEY AUTOINCREMENT,
           table_name TEXT NOT NULL,
           row_uuid   TEXT NOT NULL,
           op         TEXT NOT NULL CHECK (op IN ('upsert', 'delete')),
           ts_ms      INTEGER NOT NULL,
           origin     TEXT NOT NULL,
           origin_seq INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_sync_log_row
           ON sync_log(table_name, row_uuid, seq);
         CREATE INDEX IF NOT EXISTS idx_sync_log_origin
           ON sync_log(origin, origin_seq);
         CREATE TABLE IF NOT EXISTS sync_peers (
           device_uuid    TEXT PRIMARY KEY,
           name           TEXT NOT NULL DEFAULT '',
           cert_fp        TEXT NOT NULL,
           added_at       TEXT NOT NULL DEFAULT (datetime('now')),
           last_sync_at   TEXT,
           send_cursor    INTEGER NOT NULL DEFAULT 0,
           recv_cursor    INTEGER NOT NULL DEFAULT 0,
           needs_snapshot INTEGER NOT NULL DEFAULT 1,
           last_error     TEXT
         );
         CREATE TABLE IF NOT EXISTS sync_remote_stats (
           device_id        TEXT PRIMARY KEY,
           total_words      INTEGER NOT NULL DEFAULT 0,
           dictionary_fixes INTEGER NOT NULL DEFAULT 0,
           updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
         );
         CREATE TABLE IF NOT EXISTS sync_setting_meta (
           key    TEXT PRIMARY KEY,
           ts_ms  INTEGER NOT NULL DEFAULT 0,
           origin TEXT NOT NULL DEFAULT ''
         );",
    )?;

    // Stable identities for existing rows. Everywhere gets a fixed
    // well-known uuid so its (editable) name/style syncs as one record.
    for (table, _) in [
        ("dictionary", "id"),
        ("snippets", "id"),
        ("contexts", "id"),
        ("context_targets", "id"),
        ("context_website_targets", "id"),
        ("transcriptions", "id"),
        ("api_calls", "id"),
    ] {
        ensure_table_column(
            conn,
            table,
            "uuid",
            &format!("ALTER TABLE {table} ADD COLUMN uuid TEXT;"),
        )?;
        backfill_canonical_uuids(conn, table)?;
        conn.execute_batch(&format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_{table}_uuid ON {table}(uuid);"
        ))?;
    }
    conn.execute_batch(
        "UPDATE contexts
         SET uuid = 'everywhere-0000-0000-0000-000000000001'
         WHERE is_everywhere = 1;",
    )?;

    // Change-capture triggers. `ts_ms` is wall-clock millis; ties between two
    // devices are broken by (origin, origin_seq), which the sync engine
    // compares as a tuple. The `sync_state.applying` guard keeps engine-applied
    // remote changes from being re-captured as local edits (the engine logs
    // them itself, preserving the remote origin, so peers can dedup exactly).
    conn.execute_batch(SYNC_TRIGGER_SQL)?;
    Ok(())
}

/// Migrate AutoLearn's formerly-global state to the current Context model.
///
/// The canonical `dictionary` row remains shared and keeps its stable identity.
/// Explicit manual mappings are copied to each existing Context assignment;
/// legacy automatic mappings are retained only in an existing Everywhere
/// assignment because their originating Context was never persisted. Candidate
/// and pending rows are deliberately not copied: pre-v26 rows have no
/// originating Context, so retaining them would let ambiguous history promote a
/// correction into an arbitrary Context.  They are short-lived learning state,
/// unlike already-promoted dictionary data, which is migrated to Everywhere
/// when that is the historical assignment.
fn apply_v26_autolearn_context_migration(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "INSERT OR IGNORE INTO contexts (id, name, is_everywhere)
           VALUES (1, 'Everywhere', 1);
         CREATE TABLE IF NOT EXISTS dictionary_corrections (
           id               INTEGER PRIMARY KEY AUTOINCREMENT,
           uuid             TEXT,
           context_id       INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
           dictionary_id    INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
           mistake          TEXT NOT NULL COLLATE NOCASE,
           auto_learned     INTEGER NOT NULL DEFAULT 0 CHECK (auto_learned IN (0, 1)),
           correction_count INTEGER NOT NULL DEFAULT 0,
           confidence_tier  TEXT NOT NULL DEFAULT 'low',
           last_seen_at     DATETIME,
           created_at       DATETIME NOT NULL DEFAULT (datetime('now')),
           UNIQUE(context_id, dictionary_id, mistake)
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_dictionary_corrections_uuid
           ON dictionary_corrections(uuid) WHERE uuid IS NOT NULL;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_dictionary_corrections_context_mistake
           ON dictionary_corrections(context_id, mistake);
         CREATE INDEX IF NOT EXISTS idx_dictionary_corrections_context_dictionary
           ON dictionary_corrections(context_id, dictionary_id);",
    )?;

    // Existing installs have the old tables, whose rows cannot be attributed to
    // a Context. Rename-and-recreate is required instead of ALTER TABLE because
    // both the foreign key and the candidate uniqueness constraint changed.
    rebuild_scoped_pending_corrections(conn)?;
    rebuild_scoped_auto_learn_candidates(conn)?;
    ensure_table_column(
        conn,
        "auto_learn_events",
        "context_id",
        "ALTER TABLE auto_learn_events ADD COLUMN context_id INTEGER REFERENCES contexts(id) ON DELETE SET NULL;",
    )?;

    // v12 assigned all pre-Contexts dictionary rows to Everywhere. Heal any
    // partially migrated database with an unassigned canonical row before
    // copying its old global correction field, preserving the old behavior.
    let everywhere_id: i64 = conn.query_row(
        "SELECT id FROM contexts WHERE is_everywhere = 1 ORDER BY id LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
         SELECT ?1, d.id
           FROM dictionary d
          WHERE NOT EXISTS (
                SELECT 1 FROM dictionary_contexts dc WHERE dc.dictionary_id = d.id
          )",
        params![everywhere_id],
    )?;

    // `dictionary.mistake` was one comma-separated value for every assignment.
    // Expand it once per assigned Context. Manual rows are processed before old
    // automatic rows so a malformed legacy database cannot let an automatic
    // mapping displace a manual correction when both claim the same variant.
    // Within one authority tier, the stable dictionary id is the tie-breaker.
    struct LegacyDictionaryMigrationRow {
        dictionary_id: i64,
        mistake: Option<String>,
        auto_learned: i64,
        correction_count: i64,
        confidence_tier: String,
        last_seen_at: Option<String>,
        created_at: String,
    }

    let legacy_rows: Vec<LegacyDictionaryMigrationRow> = conn
        .prepare(
            "SELECT id, mistake, auto_learned, correction_count, confidence_tier,
                    last_seen_at, created_at
               FROM dictionary
              ORDER BY auto_learned ASC, id ASC",
        )?
        .query_map([], |row| {
            Ok(LegacyDictionaryMigrationRow {
                dictionary_id: row.get(0)?,
                mistake: row.get(1)?,
                auto_learned: row.get(2)?,
                correction_count: row.get(3)?,
                confidence_tier: row.get(4)?,
                last_seen_at: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    for row in legacy_rows {
        let LegacyDictionaryMigrationRow {
            dictionary_id,
            mistake,
            auto_learned,
            correction_count,
            confidence_tier,
            last_seen_at,
            created_at,
        } = row;
        let Some(mistake) = mistake else { continue };
        let variants = mistake
            .split(',')
            .map(str::trim)
            .filter(|variant| !variant.is_empty())
            .collect::<Vec<_>>();
        if variants.is_empty() {
            continue;
        }
        let context_sql = if auto_learned != 0 {
            // A pre-v26 automatic correction has no persisted originating
            // Context. Historically AutoLearn promoted into Everywhere, so
            // retain it there when that assignment exists; never guess that
            // an unrelated targeted assignment was its origin.
            "SELECT dc.context_id
               FROM dictionary_contexts dc
               INNER JOIN contexts c ON c.id = dc.context_id
              WHERE dc.dictionary_id = ?1 AND c.is_everywhere = 1
              ORDER BY dc.context_id"
        } else {
            // Manual sharing is explicit in dictionary_contexts, so preserve
            // the old mapping in every Context to which the user assigned it.
            "SELECT context_id FROM dictionary_contexts
              WHERE dictionary_id = ?1 ORDER BY context_id"
        };
        let context_ids: Vec<i64> = conn
            .prepare(context_sql)?
            .query_map(params![dictionary_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        for context_id in context_ids {
            for (index, variant) in variants.iter().enumerate() {
                // The old counter represented the whole comma-separated entry.
                // Keep it on one deterministic child row rather than multiplying
                // lifetime counts by the number of variants.
                let child_count = if index == 0 { correction_count } else { 0 };
                let inserted = conn.execute(
                    "INSERT OR IGNORE INTO dictionary_corrections
                       (context_id, dictionary_id, mistake, auto_learned,
                        correction_count, confidence_tier, last_seen_at, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        context_id,
                        dictionary_id,
                        variant,
                        auto_learned,
                        child_count,
                        confidence_tier,
                        last_seen_at,
                        created_at,
                    ],
                )?;
                if inserted == 0 {
                    log::warn!(
                        "db: skipped an ambiguous legacy AutoLearn correction while scoping it to Contexts"
                    );
                }
            }
        }
    }

    // From v26 onward the child table is the only mutable source of correction
    // mappings.  Leaving the old global projection populated would allow a
    // legacy query to leak one Context's correction into another.
    conn.execute("UPDATE dictionary SET mistake = NULL", [])?;
    backfill_canonical_uuids(conn, "dictionary_corrections")?;
    // v20-created databases already have the older trigger set. Re-run the
    // idempotent bundle so upgrades install the correction-row triggers too.
    conn.execute_batch(SYNC_TRIGGER_SQL)?;
    ensure_dictionary_correction_membership_trigger(conn)?;
    Ok(())
}

fn ensure_dictionary_correction_membership_trigger(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS trg_dictionary_contexts_delete_corrections
         AFTER DELETE ON dictionary_contexts BEGIN
           DELETE FROM dictionary_corrections
            WHERE context_id = OLD.context_id AND dictionary_id = OLD.dictionary_id;
         END;",
    )?;
    Ok(())
}

fn rebuild_scoped_pending_corrections(conn: &Connection) -> Result<()> {
    if table_exists(conn, "pending_corrections")?
        && !table_has_column(conn, "pending_corrections", "context_id")?
    {
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_pending_words;
             ALTER TABLE pending_corrections RENAME TO pending_corrections_legacy_v26;
             CREATE TABLE pending_corrections (
               id         INTEGER PRIMARY KEY AUTOINCREMENT,
               context_id INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
               wrong_word TEXT NOT NULL,
               correct_word TEXT NOT NULL,
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             CREATE INDEX idx_pending_words
               ON pending_corrections(context_id, wrong_word, correct_word, created_at);
             DROP TABLE pending_corrections_legacy_v26;",
        )?;
    } else {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_corrections (
               id         INTEGER PRIMARY KEY AUTOINCREMENT,
               context_id INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
               wrong_word TEXT NOT NULL,
               correct_word TEXT NOT NULL,
               created_at DATETIME NOT NULL DEFAULT (datetime('now'))
             );
             DROP INDEX IF EXISTS idx_pending_words;
             CREATE INDEX IF NOT EXISTS idx_pending_words
               ON pending_corrections(context_id, wrong_word, correct_word, created_at);",
        )?;
    }
    Ok(())
}

fn rebuild_scoped_auto_learn_candidates(conn: &Connection) -> Result<()> {
    if table_exists(conn, "auto_learn_candidates")?
        && !table_has_column(conn, "auto_learn_candidates", "context_id")?
    {
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_auto_learn_candidates_seen;
             ALTER TABLE auto_learn_candidates RENAME TO auto_learn_candidates_legacy_v26;
             CREATE TABLE auto_learn_candidates (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               context_id INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
               wrong_word TEXT NOT NULL,
               correct_word TEXT NOT NULL,
               confidence_sum REAL NOT NULL DEFAULT 0.0,
               confidence_avg REAL NOT NULL DEFAULT 0.0,
               seen_count INTEGER NOT NULL DEFAULT 0,
               last_seen_at DATETIME NOT NULL DEFAULT (datetime('now')),
               cooldown_until DATETIME,
               promoted_at DATETIME,
               UNIQUE(context_id, wrong_word, correct_word)
             );
             CREATE INDEX idx_auto_learn_candidates_seen
               ON auto_learn_candidates(context_id, last_seen_at);
             DROP TABLE auto_learn_candidates_legacy_v26;",
        )?;
    } else {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS auto_learn_candidates (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               context_id INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
               wrong_word TEXT NOT NULL,
               correct_word TEXT NOT NULL,
               confidence_sum REAL NOT NULL DEFAULT 0.0,
               confidence_avg REAL NOT NULL DEFAULT 0.0,
               seen_count INTEGER NOT NULL DEFAULT 0,
               last_seen_at DATETIME NOT NULL DEFAULT (datetime('now')),
               cooldown_until DATETIME,
               promoted_at DATETIME,
               UNIQUE(context_id, wrong_word, correct_word)
             );
             DROP INDEX IF EXISTS idx_auto_learn_candidates_seen;
             CREATE INDEX IF NOT EXISTS idx_auto_learn_candidates_seen
               ON auto_learn_candidates(context_id, last_seen_at);",
        )?;
    }
    Ok(())
}

fn ensure_sync_identity_placeholder(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_identity (uuid, name)
         SELECT lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' ||
                lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' ||
                lower(hex(randomblob(6))), ''
          WHERE NOT EXISTS (SELECT 1 FROM sync_identity)",
        [],
    )?;
    Ok(())
}

/// Keep UUIDs in the canonical hyphenated form used by `Uuid::to_string()`.
/// Older partial migrations generated bare hex strings with SQLite's
/// `randomblob`, which would compare unequal to UUIDs created by Rust.
fn backfill_canonical_uuids(conn: &Connection, table: &str) -> Result<()> {
    const EVERYWHERE_UUID: &str = "everywhere-0000-0000-0000-000000000001";
    let mut stmt = conn.prepare(&format!("SELECT rowid, uuid FROM {table}"))?;
    let repairs = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter_map(|(rowid, value)| {
            let canonical = value
                .as_deref()
                .and_then(|raw| {
                    if raw == EVERYWHERE_UUID {
                        None
                    } else {
                        Uuid::parse_str(raw).ok()
                    }
                })
                .map(|uuid| uuid.hyphenated().to_string());
            if value.as_deref() == Some(EVERYWHERE_UUID) {
                None
            } else if value.is_none() {
                Some((rowid, canonical))
            } else {
                (canonical.as_deref() != value.as_deref()).then_some((rowid, canonical))
            }
        })
        .collect::<Vec<_>>();
    drop(stmt);

    for (rowid, canonical) in repairs {
        let uuid = canonical.unwrap_or_else(|| Uuid::new_v4().to_string());
        conn.execute(
            &format!("UPDATE {table} SET uuid = ?1 WHERE rowid = ?2"),
            params![uuid, rowid],
        )?;
    }
    Ok(())
}

/// All change-capture triggers. Insert triggers on content tables first ensure
/// the row has a uuid (app code never sets one), then log - the log references
/// the row by re-reading its uuid after the backfill UPDATE. Matching UPDATE
/// triggers use `WHEN NEW.uuid IS OLD.uuid` so a uuid-backfill UPDATE (which
/// changes the uuid) is not itself captured as an edit. Junction/target
/// triggers attribute the change to the parent context, whose full aggregate
/// (row + targets + memberships) is what syncs.
const SYNC_TRIGGER_SQL: &str = "
CREATE TRIGGER IF NOT EXISTS trg_sync_dictionary_ins AFTER INSERT ON dictionary BEGIN
  UPDATE dictionary SET uuid = lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(6))) WHERE id = NEW.id AND uuid IS NULL;
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'dictionary', (SELECT uuid FROM dictionary WHERE id = NEW.id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_dictionary_upd AFTER UPDATE ON dictionary
  WHEN NEW.uuid IS OLD.uuid BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'dictionary', NEW.uuid, 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_dictionary_del AFTER DELETE ON dictionary BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'dictionary', OLD.uuid, 'delete',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
-- Correction mappings are independent sync rows. Their payload carries the
-- stable mapping UUID; the sync engine resolves the local Context and
-- canonical dictionary UUIDs when it applies the row. The Context aggregate
-- still captures membership changes through dictionary_contexts.
CREATE TRIGGER IF NOT EXISTS trg_sync_dictionary_corrections_ins
AFTER INSERT ON dictionary_corrections BEGIN
  UPDATE dictionary_corrections
     SET uuid = lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' ||
                lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' ||
                lower(hex(randomblob(6)))
   WHERE id = NEW.id AND uuid IS NULL;
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'dictionary_corrections',
         (SELECT uuid FROM dictionary_corrections WHERE id = NEW.id),
         'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                    WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
   WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
     AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_dictionary_corrections_upd
AFTER UPDATE ON dictionary_corrections
  WHEN NEW.uuid IS OLD.uuid BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'dictionary_corrections', NEW.uuid, 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                    WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
   WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
     AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_dictionary_corrections_del
AFTER DELETE ON dictionary_corrections BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'dictionary_corrections', OLD.uuid, 'delete',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                    WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
   WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
     AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_snippets_ins AFTER INSERT ON snippets BEGIN
  UPDATE snippets SET uuid = lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(6))) WHERE id = NEW.id AND uuid IS NULL;
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'snippets', (SELECT uuid FROM snippets WHERE id = NEW.id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_snippets_upd AFTER UPDATE ON snippets
  WHEN NEW.uuid IS OLD.uuid BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'snippets', NEW.uuid, 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_snippets_del AFTER DELETE ON snippets BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'snippets', OLD.uuid, 'delete',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_contexts_ins AFTER INSERT ON contexts BEGIN
  UPDATE contexts SET uuid = lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(6))) WHERE id = NEW.id AND uuid IS NULL;
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = NEW.id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_contexts_upd AFTER UPDATE ON contexts
  WHEN NEW.uuid IS OLD.uuid BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', NEW.uuid, 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_contexts_del AFTER DELETE ON contexts BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', OLD.uuid, 'delete',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_context_targets_ins AFTER INSERT ON context_targets BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = NEW.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = NEW.context_id) IS NOT NULL;
END;
-- Assigning an existing exe/domain to another context moves it: both the
-- losing and the winning aggregate changed, so both are logged.
CREATE TRIGGER IF NOT EXISTS trg_sync_context_targets_upd AFTER UPDATE ON context_targets BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = OLD.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = OLD.context_id) IS NOT NULL;
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = NEW.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = NEW.context_id) IS NOT NULL;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_context_targets_del AFTER DELETE ON context_targets BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = OLD.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = OLD.context_id) IS NOT NULL;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_context_websites_ins AFTER INSERT ON context_website_targets BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = NEW.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = NEW.context_id) IS NOT NULL;
END;
-- Same two-aggregate rule as exe targets: a moved website domain leaves one
-- context's aggregate and enters another's.
CREATE TRIGGER IF NOT EXISTS trg_sync_context_websites_upd AFTER UPDATE ON context_website_targets BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = OLD.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = OLD.context_id) IS NOT NULL;
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = NEW.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = NEW.context_id) IS NOT NULL;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_context_websites_del AFTER DELETE ON context_website_targets BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = OLD.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = OLD.context_id) IS NOT NULL;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_dictionary_contexts_ins AFTER INSERT ON dictionary_contexts BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = NEW.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = NEW.context_id) IS NOT NULL;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_dictionary_contexts_del AFTER DELETE ON dictionary_contexts BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = OLD.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = OLD.context_id) IS NOT NULL;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_snippet_contexts_ins AFTER INSERT ON snippet_contexts BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = NEW.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = NEW.context_id) IS NOT NULL;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_snippet_contexts_del AFTER DELETE ON snippet_contexts BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'contexts', (SELECT uuid FROM contexts WHERE id = OLD.context_id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT uuid FROM contexts WHERE id = OLD.context_id) IS NOT NULL;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_transcriptions_ins AFTER INSERT ON transcriptions BEGIN
  UPDATE transcriptions SET uuid = lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(6))) WHERE id = NEW.id AND uuid IS NULL;
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'transcriptions', (SELECT uuid FROM transcriptions WHERE id = NEW.id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_transcriptions_upd AFTER UPDATE ON transcriptions
  WHEN NEW.uuid IS OLD.uuid BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'transcriptions', NEW.uuid, 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_api_calls_ins AFTER INSERT ON api_calls BEGIN
  UPDATE api_calls SET uuid = lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(2))) || '-' || lower(hex(randomblob(6))) WHERE id = NEW.id AND uuid IS NULL;
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'api_calls', (SELECT uuid FROM api_calls WHERE id = NEW.id), 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_api_calls_upd AFTER UPDATE ON api_calls
  WHEN NEW.uuid IS OLD.uuid BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'api_calls', NEW.uuid, 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_api_calls_del AFTER DELETE ON api_calls BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'api_calls', OLD.uuid, 'delete',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
";

fn run_migration(conn: &mut Connection, f: impl FnOnce(&Connection) -> Result<()>) -> Result<()> {
    let tx = conn.transaction()?;
    f(&tx)?;
    tx.commit()?;
    Ok(())
}

/// The schema work of the v2 migration, shared between the `user_version < 2`
/// path (which bumps the version marker afterwards) and the self-heal path for
/// databases stranded by the legacy non-transactional v2 migration. Must run
/// inside a transaction; every statement is idempotent.
fn apply_v2_migration(conn: &Connection) -> Result<()> {
    ensure_table_column(
        conn,
        "snippets",
        "instructions",
        "ALTER TABLE snippets ADD COLUMN instructions TEXT NOT NULL DEFAULT '';",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pending_corrections (
           id           INTEGER PRIMARY KEY AUTOINCREMENT,
           wrong_word   TEXT    NOT NULL,
           correct_word TEXT    NOT NULL,
           created_at   DATETIME NOT NULL DEFAULT (datetime('now'))
         );
         CREATE INDEX IF NOT EXISTS idx_pending_words
           ON pending_corrections(wrong_word, correct_word);",
    )?;
    // Migrate dictionary to final schema: term (required) + mistake (optional).
    // No-op when the modern shape is already present.
    rebuild_legacy_dictionary(conn)?;
    Ok(())
}

/// Adds context scoping without changing the existing dictionary/snippet rows.
/// The one-time reassignment is deliberately part of the same transaction as
/// the schema work so an interrupted upgrade cannot strand content without an
/// active context.
fn apply_v12_context_migration(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS contexts (
           id            INTEGER PRIMARY KEY AUTOINCREMENT,
           name          TEXT NOT NULL COLLATE NOCASE UNIQUE,
           is_everywhere INTEGER NOT NULL DEFAULT 0 CHECK (is_everywhere IN (0, 1)),
           created_at    DATETIME NOT NULL DEFAULT (datetime('now')),
           updated_at    DATETIME NOT NULL DEFAULT (datetime('now'))
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_contexts_everywhere
           ON contexts(is_everywhere) WHERE is_everywhere = 1;
         CREATE TABLE IF NOT EXISTS context_targets (
           id           INTEGER PRIMARY KEY AUTOINCREMENT,
           context_id   INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
           executable   TEXT NOT NULL COLLATE NOCASE UNIQUE,
           platform     TEXT,
           created_at   DATETIME NOT NULL DEFAULT (datetime('now'))
         );
         CREATE INDEX IF NOT EXISTS idx_context_targets_context_id
           ON context_targets(context_id);
         CREATE TABLE IF NOT EXISTS dictionary_contexts (
           context_id    INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
           dictionary_id INTEGER NOT NULL REFERENCES dictionary(id) ON DELETE CASCADE,
           PRIMARY KEY (context_id, dictionary_id)
         );
         CREATE INDEX IF NOT EXISTS idx_dictionary_contexts_dictionary_id
           ON dictionary_contexts(dictionary_id);
         CREATE TABLE IF NOT EXISTS snippet_contexts (
           context_id INTEGER NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
           snippet_id INTEGER NOT NULL REFERENCES snippets(id) ON DELETE CASCADE,
           PRIMARY KEY (context_id, snippet_id)
         );
         CREATE INDEX IF NOT EXISTS idx_snippet_contexts_snippet_id
           ON snippet_contexts(snippet_id);
         INSERT OR IGNORE INTO contexts (id, name, is_everywhere)
           VALUES (1, 'Everywhere', 1);
         INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
           SELECT 1, id FROM dictionary;
         INSERT OR IGNORE INTO snippet_contexts (context_id, snippet_id)
           SELECT 1, id FROM snippets;",
    )?;
    Ok(())
}

/// Rebuilds `dictionary` into the modern `term`/`mistake` shape when the table
/// still has the pre-v3 `wrong`/`correct` columns. No-op when `term` already
/// exists; errors when the shape is undetectable so the caller can surface it
/// instead of silently corrupting data.
fn rebuild_legacy_dictionary(conn: &Connection) -> Result<()> {
    if table_has_column(conn, "dictionary", "term")? {
        return Ok(());
    }
    if !table_has_column(conn, "dictionary", "correct")? {
        anyhow::bail!(
            "dictionary table has neither the modern `term` column nor the legacy `wrong`/`correct` columns"
        );
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dictionary_v3 (
            id               INTEGER PRIMARY KEY AUTOINCREMENT,
            term             TEXT    NOT NULL UNIQUE,
            mistake          TEXT,
            auto_learned     INTEGER NOT NULL DEFAULT 0,
            correction_count INTEGER NOT NULL DEFAULT 0,
            created_at       DATETIME NOT NULL DEFAULT (datetime('now'))
        );
        INSERT OR IGNORE INTO dictionary_v3
            (id, term, mistake, auto_learned, correction_count, created_at)
            SELECT id,
                   COALESCE(correct, wrong),
                   CASE WHEN correct IS NOT NULL THEN wrong ELSE NULL END,
                   auto_learned, correction_count, created_at
            FROM dictionary;
        DROP TABLE dictionary;
        ALTER TABLE dictionary_v3 RENAME TO dictionary;",
    )?;
    // Complete the modern shape: the v3-era rebuild DDL predates the v4
    // columns, and a legacy table may or may not have gained them before the
    // rebuild ran. Both ensures are idempotent no-ops when already present.
    ensure_table_column(
        conn,
        "dictionary",
        "confidence_tier",
        "ALTER TABLE dictionary ADD COLUMN confidence_tier TEXT NOT NULL DEFAULT 'low';",
    )?;
    ensure_table_column(
        conn,
        "dictionary",
        "last_seen_at",
        "ALTER TABLE dictionary ADD COLUMN last_seen_at DATETIME;",
    )?;
    Ok(())
}

pub fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

pub fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&pragma)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn ensure_table_column(
    conn: &Connection,
    table: &str,
    column: &str,
    def_sql: &str,
) -> Result<bool> {
    if table_has_column(conn, table, column)? {
        return Ok(false);
    }
    conn.execute_batch(def_sql)?;
    Ok(true)
}

fn ensure_cleanup_cache_schema(conn: &Connection) -> Result<()> {
    let mut repaired = false;
    repaired |= ensure_table_column(
        conn,
        "cleanup_cache",
        "created_at_epoch",
        "ALTER TABLE cleanup_cache ADD COLUMN created_at_epoch INTEGER;",
    )?;
    repaired |= ensure_table_column(
        conn,
        "cleanup_cache",
        "last_hit_at_epoch",
        "ALTER TABLE cleanup_cache ADD COLUMN last_hit_at_epoch INTEGER;",
    )?;
    repaired |= ensure_table_column(
        conn,
        "cleanup_cache",
        "expires_at_epoch",
        "ALTER TABLE cleanup_cache ADD COLUMN expires_at_epoch INTEGER;",
    )?;
    repaired |= ensure_table_column(
        conn,
        "cleanup_cache",
        "is_snippet",
        "ALTER TABLE cleanup_cache ADD COLUMN is_snippet INTEGER NOT NULL DEFAULT 0;",
    )?;
    if repaired {
        conn.execute_batch(
            "UPDATE cleanup_cache
             SET created_at_epoch = COALESCE(created_at_epoch, CAST(strftime('%s', created_at || 'Z') AS INTEGER)),
                 last_hit_at_epoch = COALESCE(last_hit_at_epoch, CAST(strftime('%s', last_hit_at || 'Z') AS INTEGER)),
                 expires_at_epoch = COALESCE(expires_at_epoch, CAST(strftime('%s', expires_at || 'Z') AS INTEGER))
             WHERE created_at_epoch IS NULL
                OR last_hit_at_epoch IS NULL
                OR expires_at_epoch IS NULL;",
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_cleanup_cache_expires_at_epoch
           ON cleanup_cache(expires_at_epoch);
         CREATE INDEX IF NOT EXISTS idx_cleanup_cache_last_hit_at_epoch
           ON cleanup_cache(last_hit_at_epoch);",
    )?;
    Ok(())
}

fn ensure_stats_summary_triggers(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS trg_transcriptions_daily_ins
           AFTER INSERT ON transcriptions BEGIN
             INSERT INTO transcription_daily_stats (day, total_words, total_transcriptions)
             VALUES (date(NEW.created_at, 'localtime'), NEW.words, 1)
             ON CONFLICT(day) DO UPDATE SET
               total_words = total_words + excluded.total_words,
               total_transcriptions = total_transcriptions + 1;
           END;
         CREATE TRIGGER IF NOT EXISTS trg_transcriptions_daily_del
           AFTER DELETE ON transcriptions BEGIN
             UPDATE transcription_daily_stats
                SET total_words = total_words - OLD.words,
                    total_transcriptions = total_transcriptions - 1
              WHERE day = date(OLD.created_at, 'localtime');
             DELETE FROM transcription_daily_stats
              WHERE day = date(OLD.created_at, 'localtime')
                AND total_transcriptions <= 0;
           END;
         CREATE TRIGGER IF NOT EXISTS trg_transcriptions_daily_update
           AFTER UPDATE OF created_at, words ON transcriptions BEGIN
             UPDATE transcription_daily_stats
                SET total_words = total_words - OLD.words,
                    total_transcriptions = total_transcriptions - 1
              WHERE day = date(OLD.created_at, 'localtime');
             DELETE FROM transcription_daily_stats
              WHERE day = date(OLD.created_at, 'localtime')
                AND total_transcriptions <= 0;
             INSERT INTO transcription_daily_stats (day, total_words, total_transcriptions)
             VALUES (date(NEW.created_at, 'localtime'), NEW.words, 1)
             ON CONFLICT(day) DO UPDATE SET
               total_words = total_words + excluded.total_words,
               total_transcriptions = total_transcriptions + 1;
           END;
         CREATE TRIGGER IF NOT EXISTS trg_transcriptions_wpm_ins
           AFTER INSERT ON transcriptions BEGIN
             INSERT INTO lifetime_stats (id, wpm_sum, wpm_count)
             VALUES (
               1,
               CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0
                    THEN CAST(COALESCE(NEW.spoken_words, NEW.words) AS REAL) * 60000.0 / NEW.duration_ms
                    ELSE 0 END,
               CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0
                    THEN 1 ELSE 0 END
             )
             ON CONFLICT(id) DO UPDATE SET
               wpm_sum = lifetime_stats.wpm_sum + excluded.wpm_sum,
               wpm_count = lifetime_stats.wpm_count + excluded.wpm_count;
           END;
         CREATE TRIGGER IF NOT EXISTS trg_transcriptions_wpm_del
           AFTER DELETE ON transcriptions BEGIN
             UPDATE lifetime_stats
                SET wpm_sum = MAX(0, wpm_sum - CASE
                              WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0
                              THEN CAST(COALESCE(OLD.spoken_words, OLD.words) AS REAL) * 60000.0 / OLD.duration_ms
                              ELSE 0 END),
                    wpm_count = MAX(0, wpm_count - CASE
                              WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0
                              THEN 1 ELSE 0 END)
              WHERE id = 1;
           END;
         CREATE TRIGGER IF NOT EXISTS trg_transcriptions_wpm_upd
           AFTER UPDATE OF duration_ms, spoken_words, words ON transcriptions BEGIN
             UPDATE lifetime_stats
                SET wpm_sum = MAX(0, wpm_sum - CASE
                              WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0
                              THEN CAST(COALESCE(OLD.spoken_words, OLD.words) AS REAL) * 60000.0 / OLD.duration_ms
                              ELSE 0 END
                                  + CASE
                              WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0
                              THEN CAST(COALESCE(NEW.spoken_words, NEW.words) AS REAL) * 60000.0 / NEW.duration_ms
                              ELSE 0 END),
                    wpm_count = MAX(0, wpm_count
                              - CASE
                              WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0
                              THEN 1 ELSE 0 END
                              + CASE
                              WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0
                              THEN 1 ELSE 0 END)
              WHERE id = 1;
           END;",
    )?;
    Ok(())
}

/// FTS is optional at runtime because system SQLite builds may omit FTS5.
/// When present, the trigram index makes substring history search seekable.
pub(super) fn ensure_history_fts(conn: &Connection) {
    // Remove the obsolete vocabulary index. History index installation and
    // repair below publish their triggers and readiness flag together.
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS transcription_fts_meta (
           name TEXT PRIMARY KEY,
           populated INTEGER NOT NULL DEFAULT 0
         );
         DROP TRIGGER IF EXISTS trg_transcriptions_vocab_fts_ins;
         DROP TRIGGER IF EXISTS trg_transcriptions_vocab_fts_del;
         DROP TRIGGER IF EXISTS trg_transcriptions_vocab_fts_upd;
         DROP TABLE IF EXISTS transcriptions_vocab;
         DROP TABLE IF EXISTS transcriptions_vocab_fts;",
    );

    // FTS vocabulary tokenization cannot reproduce the product's word rules:
    // it splits hyphenated words and folds accents. Insights therefore keeps
    // its established streaming tokenizer; history search alone uses FTS.
    let result = (|| -> Result<()> {
        conn.execute_batch("BEGIN IMMEDIATE;")?;
        let build = (|| -> Result<()> {
            conn.execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS transcriptions_fts USING fts5(
                   raw_text, clean_text, app_name,
                   content='transcriptions', content_rowid='id',
                   tokenize='trigram'
                 );
                 INSERT OR IGNORE INTO transcription_fts_meta (name, populated)
                 VALUES ('history', 0);",
            )?;
            if !super::transcriptions::history_fts_available(conn) {
                conn.execute_batch(
                    "DROP TRIGGER IF EXISTS trg_transcriptions_fts_ins;
                     DROP TRIGGER IF EXISTS trg_transcriptions_fts_del;
                     DROP TRIGGER IF EXISTS trg_transcriptions_fts_upd;",
                )?;
                // Populate before triggers exist. The FTS rebuild command is
                // atomic with the metadata flag and makes external content
                // tables consistent without replaying application writes.
                conn.execute(
                    "INSERT INTO transcriptions_fts(transcriptions_fts) VALUES ('rebuild')",
                    [],
                )?;
                conn.execute(
                    "UPDATE transcription_fts_meta SET populated = 1 WHERE name = 'history'",
                    [],
                )?;
            }
            conn.execute_batch(
                "CREATE TRIGGER IF NOT EXISTS trg_transcriptions_fts_ins
                   AFTER INSERT ON transcriptions BEGIN
                     INSERT INTO transcriptions_fts(rowid, raw_text, clean_text, app_name)
                     VALUES (NEW.id, NEW.raw_text, NEW.clean_text, NEW.app_name);
                   END;
                 CREATE TRIGGER IF NOT EXISTS trg_transcriptions_fts_del
                   AFTER DELETE ON transcriptions BEGIN
                     INSERT INTO transcriptions_fts(transcriptions_fts, rowid, raw_text, clean_text, app_name)
                     VALUES ('delete', OLD.id, OLD.raw_text, OLD.clean_text, OLD.app_name);
                   END;
                 CREATE TRIGGER IF NOT EXISTS trg_transcriptions_fts_upd
                   AFTER UPDATE OF raw_text, clean_text, app_name ON transcriptions BEGIN
                     INSERT INTO transcriptions_fts(transcriptions_fts, rowid, raw_text, clean_text, app_name)
                     VALUES ('delete', OLD.id, OLD.raw_text, OLD.clean_text, OLD.app_name);
                     INSERT INTO transcriptions_fts(rowid, raw_text, clean_text, app_name)
                     VALUES (NEW.id, NEW.raw_text, NEW.clean_text, NEW.app_name);
                   END;",
            )?;
            Ok(())
        })();
        match build {
            Ok(()) => conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        // COMMIT itself can fail, leaving the transaction open.
        let _ = conn.execute_batch("ROLLBACK;");
        // Rollback may have restored an old readiness flag or incomplete
        // triggers. Disable both atomically before allowing unindexed writes.
        // A later successful repair must rebuild those intervening writes.
        let disabled = conn.execute_batch(
            "BEGIN IMMEDIATE;
             UPDATE transcription_fts_meta SET populated = 0 WHERE name = 'history';
             DROP TRIGGER IF EXISTS trg_transcriptions_fts_ins;
             DROP TRIGGER IF EXISTS trg_transcriptions_fts_del;
             DROP TRIGGER IF EXISTS trg_transcriptions_fts_upd;
             COMMIT;",
        );
        if let Err(disable_error) = disabled {
            let _ = conn.execute_batch("ROLLBACK;");
            log::warn!("could not disable incomplete history FTS: {disable_error}");
        }
        log::warn!("history FTS unavailable; using LIKE search: {error}");
    }
}

fn load_snippet_rows(conn: &Connection) -> Result<Vec<Snippet>> {
    let mut snippet_stmt = conn.prepare(
        "SELECT id, trigger, expansion, instructions, use_count, created_at \
         FROM snippets",
    )?;
    let rows = snippet_stmt
        .query_map([], |r| {
            Ok(Snippet {
                id: r.get(0)?,
                trigger: r.get(1)?,
                expansion: r.get(2)?,
                instructions: r.get(3)?,
                use_count: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn load_snippet_trigger_rows(conn: &Connection) -> Result<Vec<Snippet>> {
    let mut stmt = conn.prepare("SELECT id, trigger FROM snippets")?;
    let rows = stmt.query_map([], |r| {
        Ok(Snippet {
            id: r.get(0)?,
            trigger: r.get(1)?,
            expansion: String::new(),
            instructions: String::new(),
            use_count: 0,
            created_at: String::new(),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

type SnippetCacheEntry = (Weak<Mutex<Connection>>, Arc<Vec<Snippet>>);

static SPOKEN_WORD_SNIPPET_CACHE: OnceLock<Mutex<Vec<SnippetCacheEntry>>> = OnceLock::new();

fn snippet_cache() -> &'static Mutex<Vec<SnippetCacheEntry>> {
    SPOKEN_WORD_SNIPPET_CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Snippet triggers change rarely compared with transcription inserts. Keep a
/// per-connection snapshot so normal inserts do not reload every full snippet
/// row. The snapshot contains the existing `Snippet` shape because the shared
/// matcher already consumes it; it is invalidated by every snippet mutation.
fn cached_snippet_rows(db: &Db) -> Result<Arc<Vec<Snippet>>> {
    let db_ptr = Arc::as_ptr(db);
    {
        let mut cache = snippet_cache()
            .lock()
            .map_err(|_| anyhow::anyhow!("Snippet cache lock was poisoned"))?;
        cache.retain(|(owner, _)| owner.strong_count() > 0);
        if let Some((_, snippets)) = cache.iter().find(|(owner, _)| owner.as_ptr() == db_ptr) {
            return Ok(snippets.clone());
        }
    }

    // Keep the DB lock while publishing the freshly loaded snapshot. Snippet
    // writers invalidate the cache while holding the same DB lock, preventing
    // a concurrent edit from being hidden by a stale insertion into the cache.
    let conn = lock_conn(db)?;
    let snippets = Arc::new(load_snippet_trigger_rows(&conn)?);
    let mut cache = snippet_cache()
        .lock()
        .map_err(|_| anyhow::anyhow!("Snippet cache lock was poisoned"))?;
    cache.retain(|(owner, _)| owner.strong_count() > 0);
    cache.retain(|(owner, _)| owner.as_ptr() != db_ptr);
    cache.push((Arc::downgrade(db), snippets.clone()));
    Ok(snippets)
}

pub(crate) fn invalidate_snippet_cache() {
    if let Ok(mut cache) = snippet_cache().lock() {
        cache.clear();
    }
}

pub fn compute_spoken_words(db: &Db, raw_text: &str) -> Result<i64> {
    let snippets = cached_snippet_rows(db)?;
    // `cached_snippet_rows` releases the DB mutex before this matcher runs.
    Ok(crate::data::snippets::count_words_without_snippet_triggers(
        raw_text, &snippets,
    ))
}

fn backfill_spoken_words(conn: &Connection) -> Result<()> {
    let snippets = load_snippet_rows(conn)?;
    const BATCH_SIZE: i64 = 256;
    let mut last_id = 0i64;

    loop {
        let rows = {
            let mut select = conn.prepare(
                "SELECT id, raw_text FROM transcriptions
                 WHERE spoken_words IS NULL AND id > ?1
                 ORDER BY id LIMIT ?2",
            )?;
            let mapped = select.query_map(params![last_id, BATCH_SIZE], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if rows.is_empty() {
            break;
        }
        last_id = rows.last().map(|(id, _)| *id).unwrap_or(last_id);

        let updates = rows
            .iter()
            .map(|(id, raw_text)| {
                (
                    *id,
                    crate::data::snippets::count_words_without_snippet_triggers(
                        raw_text, &snippets,
                    ),
                )
            })
            .collect::<Vec<_>>();
        // Callers run migration/self-healing inside their own transaction;
        // keep each batch bounded without opening a nested transaction.
        let mut update =
            conn.prepare("UPDATE transcriptions SET spoken_words = ?2 WHERE id = ?1")?;
        for (id, spoken_words) in updates {
            update.execute(params![id, spoken_words])?;
        }
    }

    Ok(())
}
