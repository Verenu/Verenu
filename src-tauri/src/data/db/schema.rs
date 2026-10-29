#![allow(dead_code)]

use anyhow::Result;
use chrono::{Duration, Utc};
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use uuid::Uuid;

use super::*;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS diagnostic_policy (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  retention_days INTEGER NOT NULL DEFAULT 0 CHECK (retention_days BETWEEN 0 AND 365)
);
INSERT OR IGNORE INTO diagnostic_policy(id) VALUES (1);
CREATE TABLE IF NOT EXISTS diagnostic_daily_failures (
  day TEXT NOT NULL,
  subsystem TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  count INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY(day, subsystem, fingerprint)
);
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
CREATE TABLE IF NOT EXISTS backup_device_counters (
  device_id TEXT PRIMARY KEY,
  total_words INTEGER NOT NULL DEFAULT 0,
  dictionary_fixes INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS transcription_daily_stats (
  day                TEXT PRIMARY KEY,
  total_words        INTEGER NOT NULL DEFAULT 0,
  total_transcriptions INTEGER NOT NULL DEFAULT 0,
  speaking_ms        INTEGER NOT NULL DEFAULT 0,
  wpm_sum            REAL NOT NULL DEFAULT 0,
  wpm_count          INTEGER NOT NULL DEFAULT 0,
  best_wpm           REAL NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS transcription_context_daily_stats (
  day                TEXT NOT NULL,
  context_uuid       TEXT NOT NULL,
  total_words        INTEGER NOT NULL DEFAULT 0,
  total_transcriptions INTEGER NOT NULL DEFAULT 0,
  speaking_ms        INTEGER NOT NULL DEFAULT 0,
  wpm_sum            REAL NOT NULL DEFAULT 0,
  wpm_count          INTEGER NOT NULL DEFAULT 0,
  best_wpm           REAL NOT NULL DEFAULT 0,
  PRIMARY KEY (day, context_uuid)
);
                 CREATE TABLE IF NOT EXISTS stats_maintenance (
  retention_prune INTEGER NOT NULL DEFAULT 0 CHECK (retention_prune IN (0, 1))
);
INSERT OR IGNORE INTO stats_maintenance (rowid, retention_prune) VALUES (1, 0);
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
  paste_in_chunks INTEGER NOT NULL DEFAULT 0 CHECK (paste_in_chunks IN (0, 1)),
  t3_skill_mentions_disabled INTEGER NOT NULL DEFAULT 0 CHECK (t3_skill_mentions_disabled IN (0, 1)),
  contextual_formatting_override INTEGER CHECK (contextual_formatting_override IN (0, 1)),
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
CREATE TABLE IF NOT EXISTS context_sub_apps (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  uuid          TEXT NOT NULL UNIQUE,
  context_id    INTEGER REFERENCES contexts(id) ON DELETE SET NULL,
  executable    TEXT NOT NULL COLLATE NOCASE,
  app_name      TEXT,
  label         TEXT NOT NULL,
  icon          TEXT,
  title_pattern TEXT NOT NULL COLLATE NOCASE,
  match_mode    TEXT NOT NULL DEFAULT 'contains' CHECK (match_mode IN ('contains', 'starts_with', 'equals')),
  platform      TEXT,
  created_at    DATETIME NOT NULL DEFAULT (datetime('now')),
  updated_at    DATETIME NOT NULL DEFAULT (datetime('now')),
  UNIQUE (executable, title_pattern, match_mode)
);
CREATE INDEX IF NOT EXISTS idx_context_sub_apps_context_id
  ON context_sub_apps(context_id);
CREATE INDEX IF NOT EXISTS idx_context_sub_apps_executable
  ON context_sub_apps(executable);
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
CREATE TABLE IF NOT EXISTS transcription_hourly_stats (
  day TEXT NOT NULL,
  hour INTEGER NOT NULL,
  context_id INTEGER NOT NULL DEFAULT 0,
  total_words INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, hour, context_id)
);
CREATE TABLE IF NOT EXISTS provider_daily_stats (
  day TEXT NOT NULL,
  context_id INTEGER NOT NULL DEFAULT 0,
  model TEXT NOT NULL,
  provider TEXT NOT NULL,
  task TEXT NOT NULL,
  calls INTEGER NOT NULL DEFAULT 0,
  audio_ms INTEGER NOT NULL DEFAULT 0,
  input_chars INTEGER NOT NULL DEFAULT 0,
  output_chars INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, context_id, model, provider, task)
);
CREATE TABLE IF NOT EXISTS pending_transcription_contexts (
  transcription_uuid TEXT PRIMARY KEY,
  context_uuid TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS openrouter_pricing (
  model_id                TEXT PRIMARY KEY COLLATE NOCASE,
  prompt_usd_per_token    REAL NOT NULL,
  completion_usd_per_token REAL NOT NULL,
  fetched_at              INTEGER NOT NULL
);
";

pub fn open_with_recovery(path: impl AsRef<std::path::Path>) -> Result<Db> {
    match open(path.as_ref()) {
        Ok(db) => Ok(db),
        Err(first_err) => {
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
    let db_existed_before_open = db_path.exists();
    let mut conn = Connection::open(db_path)?;
    if !db_existed_before_open {
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    }
    // SQLite keeps foreign-key enforcement disabled per connection by
    // default. Context-owned correction/evidence rows use cascading foreign
    // keys, so enable enforcement before schema work or application queries.
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    anyhow::ensure!(user_version <= 39, "This database was created by a newer version of Verenu");
    if user_version < 2 && db_existed_before_open {
        let _ = std::fs::copy(db_path, db_path.with_extension("db.bak"));
    }

    conn.execute_batch(SCHEMA)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;

    if user_version < 2 {
        // sqlite3_exec leaves an explicit transaction open when a batch fails.
        // ROLLBACK restores autocommit before subsequent writes use this connection.
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
        let _ = conn.execute_batch("ROLLBACK;");
        conn.execute_batch("PRAGMA user_version = 2;")?;
    }
    if user_version == 2 {
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
    if user_version < 27 {
        log::info!("db: migrating schema {user_version} -> 27");
        run_migration(&mut conn, |conn| {
            ensure_table_column(conn, "transcription_daily_stats", "speaking_ms", "ALTER TABLE transcription_daily_stats ADD COLUMN speaking_ms INTEGER NOT NULL DEFAULT 0;")?;
            ensure_table_column(
                conn,
                "transcription_daily_stats",
                "wpm_sum",
                "ALTER TABLE transcription_daily_stats ADD COLUMN wpm_sum REAL NOT NULL DEFAULT 0;",
            )?;
            ensure_table_column(conn, "transcription_daily_stats", "wpm_count", "ALTER TABLE transcription_daily_stats ADD COLUMN wpm_count INTEGER NOT NULL DEFAULT 0;")?;
            ensure_table_column(conn, "transcription_daily_stats", "best_wpm", "ALTER TABLE transcription_daily_stats ADD COLUMN best_wpm REAL NOT NULL DEFAULT 0;")?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS stats_maintenance (
                   retention_prune INTEGER NOT NULL DEFAULT 0 CHECK (retention_prune IN (0, 1))
                 );
                 INSERT OR IGNORE INTO stats_maintenance (rowid, retention_prune) VALUES (1, 0);
                 CREATE TABLE IF NOT EXISTS transcription_context_daily_stats (
                   day TEXT NOT NULL, context_uuid TEXT NOT NULL,
                   total_words INTEGER NOT NULL DEFAULT 0,
                   total_transcriptions INTEGER NOT NULL DEFAULT 0,
                   speaking_ms INTEGER NOT NULL DEFAULT 0,
                   wpm_sum REAL NOT NULL DEFAULT 0,
                   wpm_count INTEGER NOT NULL DEFAULT 0,
                   best_wpm REAL NOT NULL DEFAULT 0,
                   PRIMARY KEY (day, context_uuid)
                 );
                 DELETE FROM transcription_context_daily_stats;
                 INSERT INTO transcription_context_daily_stats
                   (day, context_uuid, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm)
                 SELECT date(t.created_at, 'localtime'), COALESCE(c.uuid, 'unscoped'), SUM(t.words), COUNT(*), SUM(t.duration_ms),
                        SUM(CASE WHEN t.duration_ms > 0 AND COALESCE(t.spoken_words, t.words) > 0 THEN CAST(COALESCE(t.spoken_words, t.words) AS REAL) * 60000.0 / t.duration_ms ELSE 0 END),
                        SUM(CASE WHEN t.duration_ms > 0 AND COALESCE(t.spoken_words, t.words) > 0 THEN 1 ELSE 0 END),
                        MAX(CASE WHEN t.duration_ms > 0 AND COALESCE(t.spoken_words, t.words) > 0 THEN CAST(COALESCE(t.spoken_words, t.words) AS REAL) * 60000.0 / t.duration_ms ELSE 0 END)
                   FROM transcriptions t LEFT JOIN contexts c ON c.id = t.context_id
                  GROUP BY 1, 2;
                 UPDATE transcription_daily_stats
                    SET speaking_ms = COALESCE((SELECT SUM(duration_ms) FROM transcriptions t WHERE date(t.created_at, 'localtime') = day), 0),
                        wpm_sum = COALESCE((SELECT SUM(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words, words) > 0 THEN CAST(COALESCE(spoken_words, words) AS REAL) * 60000.0 / duration_ms ELSE 0 END) FROM transcriptions t WHERE date(t.created_at, 'localtime') = day), 0),
                        wpm_count = COALESCE((SELECT COUNT(*) FROM transcriptions t WHERE date(t.created_at, 'localtime') = day AND duration_ms > 0 AND COALESCE(spoken_words, words) > 0), 0),
                        best_wpm = COALESCE((SELECT MAX(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words, words) > 0 THEN CAST(COALESCE(spoken_words, words) AS REAL) * 60000.0 / duration_ms ELSE 0 END) FROM transcriptions t WHERE date(t.created_at, 'localtime') = day), 0);
                 PRAGMA user_version = 27;",
            )?;
            Ok(())
        })?;
    }
    if user_version < 28 {
        log::info!("db: migrating schema {user_version} -> 28");
        run_migration(&mut conn, |conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS transcription_hourly_stats (
                   day TEXT NOT NULL, hour INTEGER NOT NULL, context_id INTEGER NOT NULL DEFAULT 0,
                   total_words INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (day, hour, context_id)
                 );
                 CREATE TABLE IF NOT EXISTS provider_daily_stats (
                   day TEXT NOT NULL, context_id INTEGER NOT NULL DEFAULT 0, model TEXT NOT NULL,
                   provider TEXT NOT NULL, task TEXT NOT NULL, calls INTEGER NOT NULL DEFAULT 0,
                   audio_ms INTEGER NOT NULL DEFAULT 0, input_chars INTEGER NOT NULL DEFAULT 0,
                   output_chars INTEGER NOT NULL DEFAULT 0,
                   PRIMARY KEY (day, context_id, model, provider, task)
                 );
                 CREATE TABLE IF NOT EXISTS pending_transcription_contexts (
                   transcription_uuid TEXT PRIMARY KEY, context_uuid TEXT NOT NULL
                 );
                 DROP TRIGGER IF EXISTS trg_sync_api_calls_del;
                 CREATE TRIGGER trg_sync_api_calls_del AFTER DELETE ON api_calls BEGIN
                   INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
                   SELECT 'api_calls', OLD.uuid, 'delete',
                          CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
                          (SELECT uuid FROM sync_identity),
                          COALESCE((SELECT MAX(origin_seq) FROM sync_log WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
                   WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
                     AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
                     AND (SELECT COALESCE(retention_prune, 0) FROM stats_maintenance WHERE rowid = 1) = 0;
                 END;
                 PRAGMA user_version = 28;",
            )?;
            Ok(())
        })?;
    }
    if user_version < 29 {
        log::info!("db: migrating schema {user_version} -> 29");
        run_migration(&mut conn, |conn| {
            conn.execute_batch("PRAGMA user_version = 29;")?;
            Ok(())
        })?;
    }
    if user_version < 30 {
        log::info!("db: migrating schema {user_version} -> 30");
        run_migration(&mut conn, |conn| {
            let has_icon: bool = conn
                .prepare("SELECT 1 FROM pragma_table_info('context_sub_apps') WHERE name = 'icon'")?
                .exists([])?;
            if !has_icon {
                conn.execute_batch(
                    "DROP TRIGGER IF EXISTS trg_sync_context_sub_apps_ins;
                     DROP TRIGGER IF EXISTS trg_sync_context_sub_apps_upd;
                     DROP TRIGGER IF EXISTS trg_sync_context_sub_apps_del;
                     ALTER TABLE context_sub_apps RENAME TO context_sub_apps_v29;
                     CREATE TABLE context_sub_apps (
                       id            INTEGER PRIMARY KEY AUTOINCREMENT,
                       uuid          TEXT NOT NULL UNIQUE,
                       context_id    INTEGER REFERENCES contexts(id) ON DELETE SET NULL,
                       executable    TEXT NOT NULL COLLATE NOCASE,
                       app_name      TEXT,
                       label         TEXT NOT NULL,
                       icon          TEXT,
                       title_pattern TEXT NOT NULL COLLATE NOCASE,
                       match_mode    TEXT NOT NULL DEFAULT 'contains' CHECK (match_mode IN ('contains', 'starts_with', 'equals')),
                       platform      TEXT,
                       created_at    DATETIME NOT NULL DEFAULT (datetime('now')),
                       updated_at    DATETIME NOT NULL DEFAULT (datetime('now')),
                       UNIQUE (executable, title_pattern, match_mode)
                     );
                     INSERT INTO context_sub_apps
                       (id, uuid, context_id, executable, app_name, label, title_pattern, match_mode, platform, created_at, updated_at)
                     SELECT id, uuid, context_id, executable, app_name, label, title_pattern, match_mode, platform, created_at, updated_at
                       FROM context_sub_apps_v29;
                     DROP TABLE context_sub_apps_v29;
                     CREATE INDEX IF NOT EXISTS idx_context_sub_apps_context_id ON context_sub_apps(context_id);
                     CREATE INDEX IF NOT EXISTS idx_context_sub_apps_executable ON context_sub_apps(executable);",
                )?;
            }
            conn.execute_batch("PRAGMA user_version = 30;")?;
            Ok(())
        })?;
    }
    if user_version < 31 {
        run_migration(&mut conn, |conn| {
            ensure_table_column(conn, "contexts", "paste_in_chunks",
                "ALTER TABLE contexts ADD COLUMN paste_in_chunks INTEGER NOT NULL DEFAULT 0 CHECK (paste_in_chunks IN (0, 1));")?;
            conn.execute_batch("PRAGMA user_version = 31;")?;
            Ok(())
        })?;
    }
    if user_version < 32 {
        run_migration(&mut conn, |conn| {
            ensure_table_column(conn, "contexts", "t3_skill_mentions_disabled",
                "ALTER TABLE contexts ADD COLUMN t3_skill_mentions_disabled INTEGER NOT NULL DEFAULT 0 CHECK (t3_skill_mentions_disabled IN (0, 1));")?;
            conn.execute_batch("PRAGMA user_version = 32;")?;
            Ok(())
        })?;
    }
    if user_version < 33 {
        run_migration(&mut conn, |conn| {
            ensure_table_column(conn, "contexts", "contextual_formatting_override",
                "ALTER TABLE contexts ADD COLUMN contextual_formatting_override INTEGER CHECK (contextual_formatting_override IN (0, 1));")?;
            conn.execute_batch("UPDATE contexts SET contextual_formatting_override = CASE WHEN contextual_formatting_disabled = 1 THEN 1 ELSE NULL END; PRAGMA user_version = 33;")?;
            Ok(())
        })?;
    }
    if user_version < 34 {
        run_migration(&mut conn, |conn| {
            ensure_table_column(conn, "transcriptions", "activity_day", "ALTER TABLE transcriptions ADD COLUMN activity_day TEXT;")?;
            ensure_table_column(conn, "transcriptions", "activity_hour", "ALTER TABLE transcriptions ADD COLUMN activity_hour INTEGER;")?;
            ensure_table_column(conn, "api_calls", "activity_day", "ALTER TABLE api_calls ADD COLUMN activity_day TEXT;")?;
            conn.execute_batch(
                "UPDATE sync_state SET applying = 1;
                 UPDATE transcriptions SET activity_day = date(created_at, 'localtime'), activity_hour = CAST(strftime('%H', created_at, 'localtime') AS INTEGER) WHERE activity_day IS NULL OR activity_hour IS NULL;
                 UPDATE api_calls SET activity_day = date(created_at, 'localtime') WHERE activity_day IS NULL;
                 UPDATE sync_state SET applying = 0;
                 DROP TRIGGER IF EXISTS trg_sync_transcriptions_upd;
                 DROP TRIGGER IF EXISTS trg_sync_api_calls_upd;
                 CREATE TABLE IF NOT EXISTS transcription_receipts (
                   uuid TEXT NOT NULL PRIMARY KEY, day TEXT NOT NULL, hour INTEGER NOT NULL CHECK(hour BETWEEN 0 AND 23),
                   context_uuid TEXT, words INTEGER NOT NULL CHECK(words >= 0), spoken_words INTEGER CHECK(spoken_words IS NULL OR spoken_words >= 0),
                   duration_ms INTEGER NOT NULL CHECK(duration_ms >= 0), counted INTEGER NOT NULL DEFAULT 1 CHECK(counted IN (0,1))
                 );
                 CREATE TABLE IF NOT EXISTS call_receipts (
                   uuid TEXT NOT NULL PRIMARY KEY, transcription_uuid TEXT, day TEXT NOT NULL,
                   context_uuid TEXT, model TEXT NOT NULL, provider TEXT NOT NULL,
                   task TEXT NOT NULL, audio_ms INTEGER NOT NULL CHECK(audio_ms >= 0), input_chars INTEGER NOT NULL CHECK(input_chars >= 0),
                   output_chars INTEGER NOT NULL CHECK(output_chars >= 0), counted INTEGER NOT NULL DEFAULT 1 CHECK(counted IN (0,1))
                 );
                 CREATE TABLE IF NOT EXISTS legacy_pruned_days (day TEXT PRIMARY KEY);
                 CREATE TABLE IF NOT EXISTS backup_held_history (uuid TEXT NOT NULL PRIMARY KEY);
                 CREATE TABLE IF NOT EXISTS backup_preupgrade_history (uuid TEXT NOT NULL PRIMARY KEY);
                 CREATE TABLE IF NOT EXISTS backup_lifetime_wpm_contributions (source_id TEXT NOT NULL PRIMARY KEY, wpm_sum REAL NOT NULL, wpm_count INTEGER NOT NULL);
                 INSERT OR IGNORE INTO legacy_pruned_days(day)
                   SELECT s.day FROM transcription_daily_stats s
                   LEFT JOIN (SELECT date(created_at, 'localtime') AS day, COUNT(*) AS retained
                                FROM transcriptions GROUP BY 1) t ON t.day = s.day
                   WHERE s.total_transcriptions > COALESCE(t.retained, 0);
                 INSERT OR IGNORE INTO backup_preupgrade_history(uuid)
                   SELECT t.uuid FROM transcriptions t
                   LEFT JOIN transcription_daily_stats s ON s.day = t.activity_day
                   WHERE t.uuid IS NOT NULL AND (
                     s.day IS NULL OR EXISTS (SELECT 1 FROM legacy_pruned_days l WHERE l.day = t.activity_day)
                     OR s.total_transcriptions != (SELECT COUNT(*) FROM transcriptions x WHERE x.activity_day = t.activity_day)
                     OR s.total_words != (SELECT COALESCE(SUM(x.words), 0) FROM transcriptions x WHERE x.activity_day = t.activity_day)
                   );
                 PRAGMA user_version = 34;",
            )?;
            Ok(())
        })?;
    }
    if user_version < 35 {
        run_migration(&mut conn, |conn| {
            ensure_table_column(conn, "transcriptions", "website_domain", "ALTER TABLE transcriptions ADD COLUMN website_domain TEXT;")?;
            conn.execute_batch("PRAGMA user_version = 35;")?;
            Ok(())
        })?;
    }
    if user_version < 39 {
        run_migration(&mut conn, |conn| {
            ensure_table_column(conn, "transcriptions", "vocabulary_replacements", "ALTER TABLE transcriptions ADD COLUMN vocabulary_replacements INTEGER;")?;
            ensure_table_column(conn, "transcriptions", "snippet_expansions", "ALTER TABLE transcriptions ADD COLUMN snippet_expansions INTEGER;")?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS library_usage_coverage (id INTEGER PRIMARY KEY CHECK(id = 1), started_at TEXT NOT NULL);
                 INSERT OR IGNORE INTO library_usage_coverage VALUES (1, datetime('now'));
                 CREATE TABLE IF NOT EXISTS library_usage_daily (
                   kind TEXT NOT NULL CHECK(kind IN ('dictionary', 'snippet')),
                   item_uuid TEXT NOT NULL,
                   day TEXT NOT NULL,
                   hits INTEGER NOT NULL CHECK(hits > 0),
                   last_matched_at TEXT NOT NULL,
                   PRIMARY KEY(kind, item_uuid, day)
                 );
                 CREATE TABLE IF NOT EXISTS agent_change_audit (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   command TEXT NOT NULL,
                   target_id INTEGER,
                   status TEXT NOT NULL CHECK(status IN ('pending', 'succeeded', 'failed')),
                   started_at TEXT NOT NULL DEFAULT (datetime('now')),
                   finished_at TEXT
                 );
                 ",
            )?;
            conn.execute_batch("CREATE TABLE IF NOT EXISTS agent_library_changes (
              request_id TEXT PRIMARY KEY, basis TEXT NOT NULL, snapshot TEXT NOT NULL, log_seq INTEGER NOT NULL,
              undone INTEGER NOT NULL DEFAULT 0 CHECK(undone IN (0, 1)),
              created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );")?;
            ensure_table_column(conn, "agent_change_audit", "actor_id", "ALTER TABLE agent_change_audit ADD COLUMN actor_id TEXT;")?;
            conn.execute_batch("PRAGMA user_version = 39;")?;
            Ok(())
        })?;
    }
    if user_version <= 39 {
        conn.execute("DELETE FROM agent_library_changes WHERE created_at < datetime('now','-30 days')", [])?;
    }
    conn.execute("INSERT OR IGNORE INTO library_usage_coverage VALUES (1, datetime('now'))", [])?;
    conn.execute_batch(
        "CREATE VIEW IF NOT EXISTS combined_device_counters AS
           SELECT device_id, MAX(total_words) AS total_words, MAX(dictionary_fixes) AS dictionary_fixes
             FROM (SELECT device_id, total_words, dictionary_fixes FROM sync_remote_stats
                   UNION ALL SELECT device_id, total_words, dictionary_fixes FROM backup_device_counters)
            GROUP BY device_id;
         CREATE INDEX IF NOT EXISTS idx_transcriptions_context_created_at
           ON transcriptions(context_id, created_at);
         CREATE INDEX IF NOT EXISTS idx_transcriptions_activity_day
           ON transcriptions(activity_day);
         CREATE INDEX IF NOT EXISTS idx_transcription_context_daily_stats_context_day
           ON transcription_context_daily_stats(context_uuid, day);
         CREATE INDEX IF NOT EXISTS idx_transcription_receipts_day_hour
           ON transcription_receipts(day, hour);
         CREATE INDEX IF NOT EXISTS idx_call_receipts_parent
           ON call_receipts(transcription_uuid);
         CREATE INDEX IF NOT EXISTS idx_call_receipts_bucket
           ON call_receipts(day, model, provider, task);
         CREATE INDEX IF NOT EXISTS idx_pending_transcription_context
           ON pending_transcription_contexts(context_uuid);
         CREATE INDEX IF NOT EXISTS idx_pending_words
           ON pending_corrections(context_id, wrong_word, correct_word, created_at);
         CREATE INDEX IF NOT EXISTS idx_auto_learn_candidates_seen
           ON auto_learn_candidates(context_id, last_seen_at);",
    )?;
    ensure_dictionary_correction_membership_trigger(&conn)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS backup_held_calls (uuid TEXT NOT NULL PRIMARY KEY); CREATE TABLE IF NOT EXISTS backup_held_hourly_receipts (uuid TEXT NOT NULL PRIMARY KEY); CREATE TABLE IF NOT EXISTS sync_rejected_history (uuid TEXT NOT NULL PRIMARY KEY); CREATE TABLE IF NOT EXISTS sync_everywhere_aliases (uuid TEXT NOT NULL PRIMARY KEY); CREATE TABLE IF NOT EXISTS backup_context_aliases (source_uuid TEXT NOT NULL PRIMARY KEY, local_uuid TEXT NOT NULL); CREATE TABLE IF NOT EXISTS backup_context_event_sources (transcription_uuid TEXT NOT NULL, source_uuid TEXT NOT NULL, PRIMARY KEY(transcription_uuid,source_uuid));")?;
    ensure_table_column(&conn, "backup_context_aliases", "source_name", "ALTER TABLE backup_context_aliases ADD COLUMN source_name TEXT;")?;
    ensure_table_column(&conn, "backup_context_event_sources", "local_uuid", "ALTER TABLE backup_context_event_sources ADD COLUMN local_uuid TEXT;")?;
    ensure_table_column(&conn, "backup_context_event_sources", "source_name", "ALTER TABLE backup_context_event_sources ADD COLUMN source_name TEXT;")?;
    let old_context_events: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='backup_context_events')", [], |r| r.get(0))?;
    if old_context_events {
        conn.execute_batch("INSERT OR IGNORE INTO backup_context_event_sources(transcription_uuid,source_uuid) SELECT transcription_uuid,source_uuid FROM backup_context_events; DROP TABLE backup_context_events;")?;
    }
    conn.execute_batch("UPDATE backup_context_event_sources SET local_uuid=(SELECT local_uuid FROM backup_context_aliases WHERE source_uuid=backup_context_event_sources.source_uuid), source_name=(SELECT source_name FROM backup_context_aliases WHERE source_uuid=backup_context_event_sources.source_uuid) WHERE local_uuid IS NULL;")?;
    ensure_table_column(
        &conn,
        "sync_peers",
        "recv_cursor",
        "ALTER TABLE sync_peers ADD COLUMN recv_cursor INTEGER NOT NULL DEFAULT 0;",
    )?;
    ensure_sync_identity_placeholder(&conn)?;
    backfill_canonical_uuids(&conn, "dictionary_corrections")?;
    conn.execute_batch("DROP TRIGGER IF EXISTS trg_sync_transcriptions_upd; DROP TRIGGER IF EXISTS trg_sync_api_calls_upd;")?;
    conn.execute_batch(SYNC_TRIGGER_SQL)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_peer_capabilities (device_uuid TEXT PRIMARY KEY, activity_receipts INTEGER NOT NULL);
         CREATE TRIGGER IF NOT EXISTS trg_sync_transcription_receipt_ins AFTER INSERT ON transcription_receipts WHEN NEW.counted = 1 BEGIN
           INSERT INTO sync_log(table_name, row_uuid, op, ts_ms, origin, origin_seq)
           SELECT 'transcription_receipts', NEW.uuid, 'upsert', CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER), uuid,
                  COALESCE((SELECT MAX(origin_seq) FROM sync_log WHERE origin = sync_identity.uuid), 0) + 1
             FROM sync_identity WHERE (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
         END;
         CREATE TRIGGER IF NOT EXISTS trg_sync_call_receipt_ins AFTER INSERT ON call_receipts WHEN NEW.counted = 1 BEGIN
           INSERT INTO sync_log(table_name, row_uuid, op, ts_ms, origin, origin_seq)
           SELECT 'call_receipts', NEW.uuid, 'upsert', CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER), uuid,
                  COALESCE((SELECT MAX(origin_seq) FROM sync_log WHERE origin = sync_identity.uuid), 0) + 1
             FROM sync_identity WHERE (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
         END;",
    )?;
    ensure_table_column(&conn, "sync_peer_capabilities", "cleanup_cache", "ALTER TABLE sync_peer_capabilities ADD COLUMN cleanup_cache INTEGER NOT NULL DEFAULT 0;")?;
    ensure_cleanup_cache_schema(&conn)?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_api_calls_transcription_id
           ON api_calls(transcription_id);",
    )?;

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
    repair_inflated_word_counts(&mut conn)?;
    ensure_history_fts(&conn);

    Ok(Arc::new(Mutex::new(conn)))
}

const API_CALL_COMPACTION_DAYS: i64 = 90;

pub fn sqlite_disk_maintenance(db: &Db) -> Result<()> {
    compact_api_calls_older_than(db, API_CALL_COMPACTION_DAYS)?;
    let conn = lock_conn(db)?;
    let _: (i64, i64, i64) = conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |row| row.get(0))?;
    let page_count: i64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    let freelist_count: i64 = conn.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
    let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    if freelist_count < 256 || freelist_count.saturating_mul(4) < page_count || page_size <= 0 {
        return Ok(());
    }
    if auto_vacuum == 2 {
        let pages = freelist_count.min(512);
        conn.execute_batch(&format!("PRAGMA incremental_vacuum({pages});"))?;
    }
    Ok(())
}

pub fn compact_api_calls_older_than(db: &Db, max_age_days: i64) -> Result<usize> {
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    let cutoff = (Utc::now().naive_utc() - Duration::days(max_age_days.max(1)))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    tx.execute(
        "UPDATE stats_maintenance SET retention_prune = 1 WHERE rowid = 1",
        [],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO call_receipts(uuid, transcription_uuid, day, context_uuid, model, provider, task, audio_ms, input_chars, output_chars, counted)
         SELECT a.uuid, t.uuid, COALESCE(a.activity_day, date(a.created_at, 'localtime')), COALESCE(c.uuid, p.context_uuid), a.model, a.provider, a.task,
                a.audio_ms, a.input_chars, a.output_chars, NOT EXISTS(SELECT 1 FROM backup_held_calls h WHERE h.uuid = a.uuid) AND NOT EXISTS(SELECT 1 FROM backup_preupgrade_history h WHERE h.uuid = t.uuid)
           FROM api_calls a LEFT JOIN transcriptions t ON t.id = a.transcription_id
           LEFT JOIN contexts c ON c.id = t.context_id
           LEFT JOIN pending_transcription_contexts p ON p.transcription_uuid = t.uuid
          WHERE a.created_at < ?1 OR t.id IS NULL",
        [&cutoff],
    )?;
    tx.execute(
        "INSERT INTO provider_daily_stats (day, context_id, model, provider, task, calls, audio_ms, input_chars, output_chars)
         SELECT COALESCE(a.activity_day, date(a.created_at, 'localtime')), COALESCE(t.context_id, 0), a.model, a.provider, a.task,
                COUNT(*), COALESCE(SUM(a.audio_ms), 0), COALESCE(SUM(a.input_chars), 0), COALESCE(SUM(a.output_chars), 0)
           FROM api_calls a LEFT JOIN transcriptions t ON t.id = a.transcription_id
          WHERE (a.created_at < ?1 OR NOT EXISTS (SELECT 1 FROM transcriptions t2 WHERE t2.id = a.transcription_id))
            AND NOT EXISTS(SELECT 1 FROM backup_held_calls h WHERE h.uuid = a.uuid)
          GROUP BY 1, 2, 3, 4, 5
         ON CONFLICT(day, context_id, model, provider, task) DO UPDATE SET
           calls = calls + excluded.calls, audio_ms = audio_ms + excluded.audio_ms,
           input_chars = input_chars + excluded.input_chars, output_chars = output_chars + excluded.output_chars",
        [&cutoff],
    )?;
    let changed = tx.execute(
        "DELETE FROM api_calls
          WHERE created_at < ?1
             OR NOT EXISTS (SELECT 1 FROM transcriptions t WHERE t.id = api_calls.transcription_id)",
        [&cutoff],
    )?;
    tx.execute("DELETE FROM backup_held_calls WHERE uuid NOT IN (SELECT uuid FROM api_calls)", [])?;
    tx.execute(
        "UPDATE stats_maintenance SET retention_prune = 0 WHERE rowid = 1",
        [],
    )?;
    tx.commit()?;
    Ok(changed)
}

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

    conn.execute_batch(SYNC_TRIGGER_SQL)?;
    Ok(())
}

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

    rebuild_scoped_pending_corrections(conn)?;
    rebuild_scoped_auto_learn_candidates(conn)?;
    ensure_table_column(
        conn,
        "auto_learn_events",
        "context_id",
        "ALTER TABLE auto_learn_events ADD COLUMN context_id INTEGER REFERENCES contexts(id) ON DELETE SET NULL;",
    )?;

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
            "SELECT dc.context_id
               FROM dictionary_contexts dc
               INNER JOIN contexts c ON c.id = dc.context_id
              WHERE dc.dictionary_id = ?1 AND c.is_everywhere = 1
              ORDER BY dc.context_id"
        } else {
            "SELECT context_id FROM dictionary_contexts
              WHERE dictionary_id = ?1 ORDER BY context_id"
        };
        let context_ids: Vec<i64> = conn
            .prepare(context_sql)?
            .query_map(params![dictionary_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        for context_id in context_ids {
            for (index, variant) in variants.iter().enumerate() {
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

    conn.execute("UPDATE dictionary SET mistake = NULL", [])?;
    backfill_canonical_uuids(conn, "dictionary_corrections")?;
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
-- Sub-apps are their own sync table (they exist without a Context).
-- Earlier development builds logged them into the Context aggregate.
DROP TRIGGER IF EXISTS trg_sync_context_sub_apps_ins;
DROP TRIGGER IF EXISTS trg_sync_context_sub_apps_upd;
DROP TRIGGER IF EXISTS trg_sync_context_sub_apps_del;
CREATE TRIGGER IF NOT EXISTS trg_sync_sub_apps_ins AFTER INSERT ON context_sub_apps BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'context_sub_apps', NEW.uuid, 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_sub_apps_upd AFTER UPDATE ON context_sub_apps
  WHEN NEW.uuid IS OLD.uuid BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'context_sub_apps', NEW.uuid, 'upsert',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
END;
CREATE TRIGGER IF NOT EXISTS trg_sync_sub_apps_del AFTER DELETE ON context_sub_apps BEGIN
  INSERT INTO sync_log (table_name, row_uuid, op, ts_ms, origin, origin_seq)
  SELECT 'context_sub_apps', OLD.uuid, 'delete',
         CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
         (SELECT uuid FROM sync_identity),
         COALESCE((SELECT MAX(origin_seq) FROM sync_log
                   WHERE origin = (SELECT uuid FROM sync_identity)), 0) + 1
  WHERE (SELECT uuid FROM sync_identity) IS NOT NULL
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0;
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
  WHEN NEW.uuid IS OLD.uuid AND NEW.uuid IS NOT NULL BEGIN
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
  WHEN NEW.uuid IS OLD.uuid AND NEW.uuid IS NOT NULL BEGIN
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
    AND (SELECT COALESCE(applying, 0) FROM sync_state) = 0
    AND (SELECT COALESCE(retention_prune, 0) FROM stats_maintenance WHERE rowid = 1) = 0;
END;
";

fn run_migration(conn: &mut Connection, f: impl FnOnce(&Connection) -> Result<()>) -> Result<()> {
    let tx = conn.transaction()?;
    f(&tx)?;
    tx.commit()?;
    Ok(())
}

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
    rebuild_legacy_dictionary(conn)?;
    Ok(())
}

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
    conn.execute_batch("CREATE TABLE IF NOT EXISTS cleanup_cache_gate (id INTEGER PRIMARY KEY CHECK(id = 1), enabled INTEGER NOT NULL CHECK(enabled IN (0, 1))); INSERT OR IGNORE INTO cleanup_cache_gate VALUES (1, 1);")?;
    conn.execute(
        "DELETE FROM cleanup_cache WHERE key NOT LIKE 'cleanup-v2:%' AND key NOT LIKE 'cleanup-v3:%'",
        [],
    )?;
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
        "DROP TRIGGER IF EXISTS trg_transcriptions_daily_ins;
         DROP TRIGGER IF EXISTS trg_transcriptions_activity_bucket_ins;
         DROP TRIGGER IF EXISTS trg_transcriptions_activity_bucket_upd;
         DROP TRIGGER IF EXISTS trg_api_calls_activity_bucket_ins;
         CREATE TRIGGER trg_transcriptions_activity_bucket_ins AFTER INSERT ON transcriptions BEGIN
           UPDATE transcriptions SET activity_day = COALESCE(activity_day, date(NEW.created_at, 'localtime')),
             activity_hour = COALESCE(activity_hour, CAST(strftime('%H', NEW.created_at, 'localtime') AS INTEGER)) WHERE id = NEW.id;
         END;
         CREATE TRIGGER trg_transcriptions_activity_bucket_upd AFTER UPDATE OF created_at ON transcriptions
         WHEN NEW.created_at IS NOT OLD.created_at BEGIN
           UPDATE transcriptions SET activity_day = date(NEW.created_at, 'localtime'), activity_hour = CAST(strftime('%H', NEW.created_at, 'localtime') AS INTEGER) WHERE id = NEW.id;
         END;
         CREATE TRIGGER trg_api_calls_activity_bucket_ins AFTER INSERT ON api_calls BEGIN
           UPDATE api_calls SET activity_day = COALESCE(activity_day, date(NEW.created_at, 'localtime')) WHERE id = NEW.id;
         END;
         DROP TRIGGER IF EXISTS trg_transcriptions_daily_del;
         DROP TRIGGER IF EXISTS trg_transcriptions_daily_update;
         DROP TRIGGER IF EXISTS trg_transcriptions_context_daily_ins;
         DROP TRIGGER IF EXISTS trg_transcriptions_context_daily_del;
         DROP TRIGGER IF EXISTS trg_transcriptions_context_daily_update;
         DROP TRIGGER IF EXISTS trg_transcriptions_wpm_ins;
         DROP TRIGGER IF EXISTS trg_transcriptions_wpm_del;
         DROP TRIGGER IF EXISTS trg_transcriptions_wpm_upd;
         CREATE TRIGGER trg_transcriptions_daily_ins
           AFTER INSERT ON transcriptions WHEN NOT EXISTS (SELECT 1 FROM backup_held_history WHERE uuid = NEW.uuid) BEGIN
             INSERT INTO transcription_daily_stats (day, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm)
             VALUES (COALESCE(NEW.activity_day, date(NEW.created_at, 'localtime')), NEW.words, 1, NEW.duration_ms,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN CAST(COALESCE(NEW.spoken_words, NEW.words) AS REAL) * 60000.0 / NEW.duration_ms ELSE 0 END,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN 1 ELSE 0 END,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN CAST(COALESCE(NEW.spoken_words, NEW.words) AS REAL) * 60000.0 / NEW.duration_ms ELSE 0 END)
             ON CONFLICT(day) DO UPDATE SET
               total_words = total_words + excluded.total_words,
               total_transcriptions = total_transcriptions + 1,
               speaking_ms = speaking_ms + excluded.speaking_ms,
               wpm_sum = wpm_sum + excluded.wpm_sum,
               wpm_count = wpm_count + excluded.wpm_count,
               best_wpm = MAX(best_wpm, excluded.best_wpm);
           END;
         CREATE TRIGGER trg_transcriptions_context_daily_ins
           AFTER INSERT ON transcriptions WHEN NOT EXISTS (SELECT 1 FROM backup_held_history WHERE uuid = NEW.uuid) BEGIN
             INSERT INTO transcription_context_daily_stats
               (day, context_uuid, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm)
             VALUES (COALESCE(NEW.activity_day, date(NEW.created_at, 'localtime')), COALESCE((SELECT uuid FROM contexts WHERE id = NEW.context_id), 'unscoped'), NEW.words, 1, NEW.duration_ms,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN CAST(COALESCE(NEW.spoken_words, NEW.words) AS REAL) * 60000.0 / NEW.duration_ms ELSE 0 END,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN 1 ELSE 0 END,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN CAST(COALESCE(NEW.spoken_words, NEW.words) AS REAL) * 60000.0 / NEW.duration_ms ELSE 0 END)
             ON CONFLICT(day, context_uuid) DO UPDATE SET
               total_words = total_words + excluded.total_words,
               total_transcriptions = total_transcriptions + 1,
               speaking_ms = speaking_ms + excluded.speaking_ms,
               wpm_sum = wpm_sum + excluded.wpm_sum,
               wpm_count = wpm_count + excluded.wpm_count,
               best_wpm = MAX(best_wpm, excluded.best_wpm);
           END;
         CREATE TRIGGER IF NOT EXISTS trg_transcriptions_daily_del
           AFTER DELETE ON transcriptions WHEN NOT EXISTS (SELECT 1 FROM backup_held_history WHERE uuid = OLD.uuid) BEGIN
             UPDATE transcription_daily_stats
                SET total_words = total_words - OLD.words,
                    total_transcriptions = total_transcriptions - 1,
                    speaking_ms = speaking_ms - OLD.duration_ms,
                    wpm_sum = wpm_sum - CASE WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0 THEN CAST(COALESCE(OLD.spoken_words, OLD.words) AS REAL) * 60000.0 / OLD.duration_ms ELSE 0 END,
                    wpm_count = wpm_count - CASE WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0 THEN 1 ELSE 0 END
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND NOT EXISTS (SELECT 1 FROM stats_maintenance WHERE retention_prune = 1);
             DELETE FROM transcription_daily_stats
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND total_transcriptions <= 0
                AND NOT EXISTS (SELECT 1 FROM stats_maintenance WHERE retention_prune = 1);
             UPDATE transcription_daily_stats
                SET best_wpm = MAX(COALESCE((SELECT MAX(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words, words) > 0 THEN CAST(COALESCE(spoken_words, words) AS REAL) * 60000.0 / duration_ms ELSE 0 END) FROM transcriptions WHERE NOT EXISTS (SELECT 1 FROM backup_held_history h WHERE h.uuid = transcriptions.uuid) AND COALESCE(activity_day, date(created_at, 'localtime')) = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))), 0), COALESCE((SELECT MAX(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words,words) > 0 THEN CAST(COALESCE(spoken_words,words) AS REAL)*60000.0/duration_ms ELSE 0 END) FROM transcription_receipts WHERE counted=1 AND day=COALESCE(OLD.activity_day,date(OLD.created_at,'localtime'))),0))
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND NOT EXISTS (SELECT 1 FROM stats_maintenance WHERE retention_prune = 1);
           END;
         CREATE TRIGGER trg_transcriptions_context_daily_del
           AFTER DELETE ON transcriptions WHEN NOT EXISTS (SELECT 1 FROM backup_held_history WHERE uuid = OLD.uuid) BEGIN
             UPDATE transcription_context_daily_stats
                SET total_words = total_words - OLD.words,
                    total_transcriptions = total_transcriptions - 1,
                    speaking_ms = speaking_ms - OLD.duration_ms,
                    wpm_sum = wpm_sum - CASE WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0 THEN CAST(COALESCE(OLD.spoken_words, OLD.words) AS REAL) * 60000.0 / OLD.duration_ms ELSE 0 END,
                    wpm_count = wpm_count - CASE WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0 THEN 1 ELSE 0 END
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND context_uuid = COALESCE((SELECT uuid FROM contexts WHERE id = OLD.context_id), 'unscoped')
                AND NOT EXISTS (SELECT 1 FROM stats_maintenance WHERE retention_prune = 1);
             DELETE FROM transcription_context_daily_stats
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND context_uuid = COALESCE((SELECT uuid FROM contexts WHERE id = OLD.context_id), 'unscoped')
                AND total_transcriptions <= 0
                AND NOT EXISTS (SELECT 1 FROM stats_maintenance WHERE retention_prune = 1);
             UPDATE transcription_context_daily_stats
                SET best_wpm = MAX(COALESCE((SELECT MAX(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words, words) > 0 THEN CAST(COALESCE(spoken_words, words) AS REAL) * 60000.0 / duration_ms ELSE 0 END) FROM transcriptions WHERE NOT EXISTS (SELECT 1 FROM backup_held_history h WHERE h.uuid = transcriptions.uuid) AND COALESCE(activity_day, date(created_at, 'localtime')) = day AND COALESCE((SELECT uuid FROM contexts WHERE id = context_id), 'unscoped') = context_uuid), 0), COALESCE((SELECT MAX(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words,words) > 0 THEN CAST(COALESCE(spoken_words,words) AS REAL)*60000.0/duration_ms ELSE 0 END) FROM transcription_receipts r WHERE r.counted=1 AND r.day=transcription_context_daily_stats.day AND (CASE WHEN EXISTS(SELECT 1 FROM pending_transcription_contexts p WHERE p.transcription_uuid=r.uuid) THEN 'unscoped' ELSE COALESCE(r.context_uuid,'unscoped') END)=transcription_context_daily_stats.context_uuid),0))
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND context_uuid = COALESCE((SELECT uuid FROM contexts WHERE id = OLD.context_id), 'unscoped')
                AND NOT EXISTS (SELECT 1 FROM stats_maintenance WHERE retention_prune = 1);
           END;
         CREATE TRIGGER trg_transcriptions_context_daily_update
           AFTER UPDATE OF created_at, words, duration_ms, spoken_words, context_id ON transcriptions WHEN NOT EXISTS (SELECT 1 FROM backup_held_history WHERE uuid = NEW.uuid) BEGIN
             UPDATE transcription_context_daily_stats
                SET total_words = total_words - OLD.words,
                    total_transcriptions = total_transcriptions - 1,
                    speaking_ms = speaking_ms - OLD.duration_ms,
                    wpm_sum = wpm_sum - CASE WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0 THEN CAST(COALESCE(OLD.spoken_words, OLD.words) AS REAL) * 60000.0 / OLD.duration_ms ELSE 0 END,
                    wpm_count = wpm_count - CASE WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0 THEN 1 ELSE 0 END
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND context_uuid = COALESCE((SELECT uuid FROM contexts WHERE id = OLD.context_id), 'unscoped');
             DELETE FROM transcription_context_daily_stats
              WHERE total_transcriptions <= 0
                AND day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND context_uuid = COALESCE((SELECT uuid FROM contexts WHERE id = OLD.context_id), 'unscoped');
             UPDATE transcription_context_daily_stats
                SET best_wpm = MAX(COALESCE((SELECT MAX(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words, words) > 0 THEN CAST(COALESCE(spoken_words, words) AS REAL) * 60000.0 / duration_ms ELSE 0 END) FROM transcriptions WHERE NOT EXISTS (SELECT 1 FROM backup_held_history h WHERE h.uuid = transcriptions.uuid) AND COALESCE(activity_day, date(created_at, 'localtime')) = day AND COALESCE((SELECT uuid FROM contexts WHERE id = context_id), 'unscoped') = context_uuid), 0), COALESCE((SELECT MAX(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words,words) > 0 THEN CAST(COALESCE(spoken_words,words) AS REAL)*60000.0/duration_ms ELSE 0 END) FROM transcription_receipts r WHERE r.counted=1 AND r.day=transcription_context_daily_stats.day AND (CASE WHEN EXISTS(SELECT 1 FROM pending_transcription_contexts p WHERE p.transcription_uuid=r.uuid) THEN 'unscoped' ELSE COALESCE(r.context_uuid,'unscoped') END)=transcription_context_daily_stats.context_uuid),0))
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND context_uuid = COALESCE((SELECT uuid FROM contexts WHERE id = OLD.context_id), 'unscoped')
                AND NOT EXISTS (SELECT 1 FROM stats_maintenance WHERE retention_prune = 1);
             INSERT INTO transcription_context_daily_stats
               (day, context_uuid, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm)
             VALUES (CASE WHEN NEW.created_at IS NOT OLD.created_at THEN date(NEW.created_at, 'localtime') ELSE COALESCE(NEW.activity_day, date(NEW.created_at, 'localtime')) END, COALESCE((SELECT uuid FROM contexts WHERE id = NEW.context_id), 'unscoped'), NEW.words, 1, NEW.duration_ms,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN CAST(COALESCE(NEW.spoken_words, NEW.words) AS REAL) * 60000.0 / NEW.duration_ms ELSE 0 END,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN 1 ELSE 0 END,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN CAST(COALESCE(NEW.spoken_words, NEW.words) AS REAL) * 60000.0 / NEW.duration_ms ELSE 0 END)
             ON CONFLICT(day, context_uuid) DO UPDATE SET
               total_words = total_words + excluded.total_words,
               total_transcriptions = total_transcriptions + 1,
               speaking_ms = speaking_ms + excluded.speaking_ms,
               wpm_sum = wpm_sum + excluded.wpm_sum,
               wpm_count = wpm_count + excluded.wpm_count,
               best_wpm = MAX(best_wpm, excluded.best_wpm);
           END;
         CREATE TRIGGER IF NOT EXISTS trg_transcriptions_daily_update
           AFTER UPDATE OF created_at, words, duration_ms, spoken_words ON transcriptions WHEN NOT EXISTS (SELECT 1 FROM backup_held_history WHERE uuid = NEW.uuid) BEGIN
             UPDATE transcription_daily_stats
                SET total_words = total_words - OLD.words,
                    total_transcriptions = total_transcriptions - 1,
                    speaking_ms = speaking_ms - OLD.duration_ms,
                    wpm_sum = wpm_sum - CASE WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0 THEN CAST(COALESCE(OLD.spoken_words, OLD.words) AS REAL) * 60000.0 / OLD.duration_ms ELSE 0 END,
                    wpm_count = wpm_count - CASE WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0 THEN 1 ELSE 0 END
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'));
             DELETE FROM transcription_daily_stats
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND total_transcriptions <= 0;
             UPDATE transcription_daily_stats
                SET best_wpm = MAX(COALESCE((SELECT MAX(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words, words) > 0 THEN CAST(COALESCE(spoken_words, words) AS REAL) * 60000.0 / duration_ms ELSE 0 END) FROM transcriptions WHERE NOT EXISTS (SELECT 1 FROM backup_held_history h WHERE h.uuid = transcriptions.uuid) AND COALESCE(activity_day, date(created_at, 'localtime')) = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))), 0), COALESCE((SELECT MAX(CASE WHEN duration_ms > 0 AND COALESCE(spoken_words,words) > 0 THEN CAST(COALESCE(spoken_words,words) AS REAL)*60000.0/duration_ms ELSE 0 END) FROM transcription_receipts WHERE counted=1 AND day=COALESCE(OLD.activity_day,date(OLD.created_at,'localtime'))),0))
              WHERE day = COALESCE(OLD.activity_day, date(OLD.created_at, 'localtime'))
                AND NOT EXISTS (SELECT 1 FROM stats_maintenance WHERE retention_prune = 1);
             INSERT INTO transcription_daily_stats (day, total_words, total_transcriptions, speaking_ms, wpm_sum, wpm_count, best_wpm)
             VALUES (CASE WHEN NEW.created_at IS NOT OLD.created_at THEN date(NEW.created_at, 'localtime') ELSE COALESCE(NEW.activity_day, date(NEW.created_at, 'localtime')) END, NEW.words, 1, NEW.duration_ms,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN CAST(COALESCE(NEW.spoken_words, NEW.words) AS REAL) * 60000.0 / NEW.duration_ms ELSE 0 END,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN 1 ELSE 0 END,
                     CASE WHEN NEW.duration_ms > 0 AND COALESCE(NEW.spoken_words, NEW.words) > 0 THEN CAST(COALESCE(NEW.spoken_words, NEW.words) AS REAL) * 60000.0 / NEW.duration_ms ELSE 0 END)
             ON CONFLICT(day) DO UPDATE SET
               total_words = total_words + excluded.total_words,
               total_transcriptions = total_transcriptions + 1,
               speaking_ms = speaking_ms + excluded.speaking_ms,
               wpm_sum = wpm_sum + excluded.wpm_sum,
               wpm_count = wpm_count + excluded.wpm_count,
               best_wpm = MAX(best_wpm, excluded.best_wpm);
           END;
         CREATE TRIGGER IF NOT EXISTS trg_transcriptions_wpm_ins
           AFTER INSERT ON transcriptions WHEN NOT EXISTS (SELECT 1 FROM backup_held_history WHERE uuid = NEW.uuid) BEGIN
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
           AFTER DELETE ON transcriptions WHEN NOT EXISTS (SELECT 1 FROM backup_held_history WHERE uuid = OLD.uuid) BEGIN
             UPDATE lifetime_stats
                SET wpm_sum = MAX(0, wpm_sum - CASE
                              WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0
                              THEN CAST(COALESCE(OLD.spoken_words, OLD.words) AS REAL) * 60000.0 / OLD.duration_ms
                              ELSE 0 END),
                    wpm_count = MAX(0, wpm_count - CASE
                              WHEN OLD.duration_ms > 0 AND COALESCE(OLD.spoken_words, OLD.words) > 0
                              THEN 1 ELSE 0 END)
              WHERE id = 1
                AND NOT EXISTS (SELECT 1 FROM stats_maintenance WHERE retention_prune = 1);
           END;
         CREATE TRIGGER IF NOT EXISTS trg_transcriptions_wpm_upd
           AFTER UPDATE OF duration_ms, spoken_words, words ON transcriptions WHEN NOT EXISTS (SELECT 1 FROM backup_held_history WHERE uuid = NEW.uuid) BEGIN
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
        let _ = conn.execute_batch("ROLLBACK;");
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
                inherited_from_everywhere: false,
                assigned_to_context: false,
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
            inherited_from_everywhere: false,
            assigned_to_context: false,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

type SnippetCacheEntry = (Weak<Mutex<Connection>>, Arc<Vec<Snippet>>);

static SPOKEN_WORD_SNIPPET_CACHE: OnceLock<Mutex<Vec<SnippetCacheEntry>>> = OnceLock::new();

fn snippet_cache() -> &'static Mutex<Vec<SnippetCacheEntry>> {
    SPOKEN_WORD_SNIPPET_CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

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
        let mut update =
            conn.prepare("UPDATE transcriptions SET spoken_words = ?2 WHERE id = ?1")?;
        for (id, spoken_words) in updates {
            update.execute(params![id, spoken_words])?;
        }
    }

    Ok(())
}

pub(crate) fn repair_inflated_word_counts(conn: &mut Connection) -> Result<()> {
    backfill_spoken_words(conn)?;
    let needs_repair: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM transcriptions
          WHERE spoken_words IS NOT NULL AND words > spoken_words)",
        [],
        |r| r.get(0),
    )?;
    if !needs_repair {
        return Ok(());
    }
    let tx = conn.transaction()?;
    let overcount: i64 = tx.query_row(
        "SELECT COALESCE(SUM(words - spoken_words), 0) FROM transcriptions
          WHERE spoken_words IS NOT NULL AND words > spoken_words",
        [],
        |r| r.get(0),
    )?;
    tx.execute(
        "UPDATE transcriptions SET words = spoken_words
          WHERE spoken_words IS NOT NULL AND words > spoken_words",
        [],
    )?;
    if overcount > 0 {
        tx.execute(
            "UPDATE lifetime_stats SET total_words = MAX(0, total_words - ?1) WHERE id = 1",
            params![overcount],
        )?;
    }
    tx.commit()?;
    Ok(())
}
