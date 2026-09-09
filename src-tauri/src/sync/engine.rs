//! The sync engine: turns the local change log into wire ops, applies remote
//! ops with last-writer-wins semantics, and drives a full sync session over an
//! established (already-authenticated) framed message stream.
//!
//! Conflict model, in one place:
//! - Every row change carries a stamp (wall-clock ms, origin device uuid,
//!   origin sequence). Stamps are totally ordered, so every device converges.
//! - Upsert vs upsert / upsert vs delete: higher stamp wins (LWW per record).
//! - Natural-key collisions (two devices created "the same" dictionary term,
//!   snippet trigger, or context name independently): the row with the higher
//!   stamp survives and the loser is hard-deleted on both sides. When the
//!   local row wins, an anti-entropy tombstone is logged for the remote uuid
//!   so the originating device removes its losing row too.
//! - Contexts sync as aggregates (row + targets + memberships); the whole
//!   aggregate is LWW, so two devices editing the same context concurrently
//!   converge on the later edit wholesale.
//! - Context-owned dictionary corrections sync as independent child rows,
//!   keyed by their own UUID and carrying Context/canonical UUID references;
//!   transient AutoLearn candidates and pending evidence remain local-only.
//! - Natural-key loser tombstones use a reserved sync-log namespace so an
//!   in-flight child can distinguish canonical replacement from explicit
//!   deletion without adding another database schema column.
//! - Settings are LWW per key using `sync_setting_meta` stamps.
//! - Lifetime counters are summed per device (each dictation is counted by
//!   exactly the device it happened on), so totals merge without double-count.

use anyhow::{anyhow, Context as _, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::{db, store};
use crate::DbHandle;

use super::protocol::{
    read_message, send_message, DeviceStatsDto, DictionaryCorrectionRow, Hello, Message, OpsBatch,
    PullRequest, SettingRecord, StatsExchange, SyncOp, OPS_PER_BATCH, PROTOCOL_VERSION,
    SNAPSHOT_ROW_CHUNK,
};
use super::store as sync_store;
use super::store::SyncPeer;

/// Where a snapshot send has gotten to. Held by the sender across batches.
#[derive(Debug, Default, Clone, Copy)]
pub struct SnapshotProgress {
    /// 0 = dictionary, 1 = snippets, 2 = contexts, 3 = dictionary corrections,
    /// 4 = transcriptions, 5 = api_calls, 6 = retained tombstones, 7 = done.
    pub stage: u8,
    pub last_id: i64,
    /// Sequence namespace for synthesized snapshot stamps. Snapshot rows are
    /// not written to the local log, so this must survive across batches.
    pub origin_seq: Option<i64>,
}

/// Settings keys that sync between paired devices. Everything else in
/// settings.json is device-local by design - see docs/lan-sync.md for the
/// full classification. API keys never appear here (they are not even stored
/// in settings.json; they live in the OS credential store).
pub const SYNCABLE_SETTINGS: &[&str] = &[
    store::TRANSCRIPTION_PROVIDER,
    store::TRANSCRIPTION_LANGUAGE,
    store::CLEANUP_PROVIDER,
    store::TRANSCRIPTION_MODEL,
    store::CLEANUP_MODEL,
    store::TRANSCRIPTION_MODELS_BY_PROVIDER,
    store::CLEANUP_MODELS_BY_PROVIDER,
    store::TRANSCRIPTION_DEFAULT_MODEL,
    store::CLEANUP_DEFAULT_MODEL,
    store::TRANSCRIPTION_FALLBACK_MODELS,
    store::CLEANUP_FALLBACK_MODELS,
    store::DUAL_TRANSCRIPTION_ENABLED,
    store::CLEANUP_ENABLED,
    store::DEFAULT_TONE,
    store::CLEANUP_INTENSITY,
    store::APP_CONTEXT_HINT,
    store::AUTO_LEARN_ENABLED,
    store::AUTO_LEARN_EVENT_MODE,
    // CONTEXTUAL_CAPS and AUTO_SPACING are legacy mirrors of this canonical
    // key. They are written together when CONTEXTUAL_FORMATTING changes and
    // must not be independently LWW-merged.
    store::CONTEXTUAL_FORMATTING,
    store::CLEANUP_PROMPT_OVERRIDE,
    store::VERENU_SERVICE_CHECKS_ENABLED,
];

const UNRESOLVED_APP_PREFIX: &str = "?::";
const NATURAL_KEY_TOMBSTONE_TABLE: &str = "dictionary_natural_key";

type ExistingCorrection = (i64, Option<String>, bool, i64, String, Option<String>);
type CorrectionTarget = (i64, Option<String>, i64, bool);
type CorrectionIdentity = (i64, i64, i64, String, bool);

/// The environment the engine needs beyond the database. The Tauri manager
/// implements it against the real app; tests use an in-memory stand-in so the
/// whole session flow is testable without a running app.
pub trait SyncHost: Send + Sync {
    fn device_uuid(&self) -> String;
    fn device_name(&self) -> String;
    fn app_version(&self) -> String;
    /// Values + stamps for all [`SYNCABLE_SETTINGS`], from settings.json.
    fn settings_payload(&self) -> Result<Vec<SettingRecord>>;
    /// Persist a remote setting value and run its local side effects.
    fn apply_remote_setting(&self, key: &str, value: &serde_json::Value) -> Result<(), String>;
    /// Persist a group of remote settings as one document write.  The default
    /// keeps test and third-party hosts source-compatible; the real host
    /// overrides it to make the batch atomic and avoid one rewrite per key.
    fn apply_remote_settings(
        &self,
        settings: &[(String, serde_json::Value)],
    ) -> Result<(), String> {
        for (key, value) in settings {
            self.apply_remote_setting(key, value)?;
        }
        Ok(())
    }
    /// Maps a peer's platform-specific app identifier to this device. `None`
    /// preserves the assignment as unresolved instead of binding the wrong app.
    fn resolve_app_target(&self, source: &str) -> Option<String> {
        Some(source.to_string())
    }
    fn resolve_app_target_with_metadata(
        &self,
        source: &str,
        app_name: Option<&str>,
        developer: Option<&str>,
    ) -> Option<(String, Option<String>, Option<String>)> {
        self.resolve_app_target(source).map(|resolved| {
            (
                resolved,
                app_name.map(str::to_string),
                developer.map(str::to_string),
            )
        })
    }
}

pub fn unresolved_app_target(source: &str) -> String {
    format!(
        "{UNRESOLVED_APP_PREFIX}{}",
        source.trim_start_matches(UNRESOLVED_APP_PREFIX)
    )
}

fn resolve_context_targets_in_ops(host: &dyn SyncHost, ops: &[SyncOp]) -> Vec<SyncOp> {
    ops.iter()
        .cloned()
        .map(|mut op| {
            if op.table != "contexts" || op.op == "delete" {
                return op;
            }
            if let Some(targets) = op
                .payload
                .as_mut()
                .and_then(|payload| payload.get_mut("targets"))
                .and_then(serde_json::Value::as_array_mut)
            {
                for target in targets {
                    // Wire shape is either a bare string (a peer on an older
                    // build, or a legacy pre-platform-tag row) or a
                    // `{executable, platform, app_name, developer}` object. Either way, rewrite
                    // `executable` to this device's own app identifier (or the
                    // `?::` unresolved marker) and stamp `platform` with this
                    // device's OS once resolved, since the rewritten string
                    // now follows this platform's naming convention.
                    let (source, app_name, developer) = match target {
                        serde_json::Value::String(s) => (Some(s.clone()), None, None),
                        serde_json::Value::Object(map) => (
                            map.get("executable")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            map.get("app_name")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            map.get("developer")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        ),
                        _ => (None, None, None),
                    };
                    let Some(source) = source else { continue };
                    let (resolved, resolved_name, resolved_developer) = host
                        .resolve_app_target_with_metadata(
                            &source,
                            app_name.as_deref(),
                            developer.as_deref(),
                        )
                        .unwrap_or_else(|| {
                            (
                                unresolved_app_target(&source),
                                app_name.clone(),
                                developer.clone(),
                            )
                        });
                    let platform = if resolved.starts_with(UNRESOLVED_APP_PREFIX) {
                        None
                    } else {
                        db::current_platform_tag()
                    };
                    *target = serde_json::json!({
                        "executable": resolved,
                        "platform": platform,
                        "app_name": resolved_name,
                        "developer": resolved_developer,
                    });
                }
            }
            op
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Payload row types (the JSON shapes that travel inside SyncOp payloads)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct DictionaryRow {
    pub term: String,
    #[serde(default)]
    pub mistake: Option<String>,
    #[serde(default)]
    pub auto_learned: bool,
    #[serde(default)]
    pub correction_count: i64,
    #[serde(default = "default_confidence_tier")]
    pub confidence_tier: String,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    pub created_at: String,
}

fn default_confidence_tier() -> String {
    "low".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SnippetRow {
    pub trigger: String,
    pub expansion: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub use_count: i64,
    pub created_at: String,
}

/// A synced exe target, carrying which OS assigned it. `#[serde(untagged)]`
/// so a peer still running a pre-platform-tagging build (bare string wire
/// format) deserializes fine, just with `platform: None`.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum TargetEntry {
    Legacy(String),
    Tagged {
        executable: String,
        #[serde(default)]
        platform: Option<String>,
        #[serde(default)]
        app_name: Option<String>,
        #[serde(default)]
        developer: Option<String>,
    },
}

impl TargetEntry {
    fn executable(&self) -> &str {
        match self {
            TargetEntry::Legacy(exe) => exe,
            TargetEntry::Tagged { executable, .. } => executable,
        }
    }

    fn platform(&self) -> Option<&str> {
        match self {
            TargetEntry::Legacy(_) => None,
            TargetEntry::Tagged { platform, .. } => platform.as_deref(),
        }
    }

    fn app_name(&self) -> Option<&str> {
        match self {
            TargetEntry::Legacy(_) => None,
            TargetEntry::Tagged { app_name, .. } => app_name.as_deref(),
        }
    }

    fn developer(&self) -> Option<&str> {
        match self {
            TargetEntry::Legacy(_) => None,
            TargetEntry::Tagged { developer, .. } => developer.as_deref(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContextAggregate {
    pub name: String,
    #[serde(default)]
    pub is_everywhere: bool,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub cleanup_intensity: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub custom_instructions: Option<String>,
    #[serde(default)]
    pub contextual_formatting_disabled: bool,
    #[serde(default)]
    pub pinned_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub targets: Vec<TargetEntry>,
    #[serde(default)]
    pub websites: Vec<String>,
    #[serde(default)]
    pub dictionary_uuids: Vec<String>,
    /// Dictionary references retain the natural term alongside the stable
    /// UUID. This lets a Context membership follow a canonical dictionary
    /// natural-key conflict after the losing UUID has been reparented.
    #[serde(default)]
    pub dictionary_entries: Vec<DictionaryReference>,
    #[serde(default)]
    pub snippet_uuids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DictionaryReference {
    pub uuid: String,
    #[serde(default)]
    pub term: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TranscriptionRow {
    pub raw_text: String,
    pub clean_text: String,
    #[serde(default)]
    pub words: i64,
    #[serde(default)]
    pub spoken_words: Option<i64>,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub api_used: String,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub context_uuid: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiCallRow {
    #[serde(default)]
    pub transcription_uuid: Option<String>,
    pub model: String,
    pub provider: String,
    pub task: String,
    #[serde(default)]
    pub audio_ms: i64,
    #[serde(default)]
    pub input_chars: i64,
    #[serde(default)]
    pub output_chars: i64,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Apply bookkeeping
// ---------------------------------------------------------------------------

/// What changed when a batch was applied, for frontend refresh events.
#[derive(Debug, Default, Clone)]
pub struct ApplySummary {
    pub dictionary: bool,
    /// Persistent Context-scoped correction mappings are separate sync rows,
    /// but also set `dictionary` so older consumers refresh their dictionary
    /// cache. The distinct flag lets Context-aware consumers refresh their
    /// materialized view without guessing from the canonical table.
    pub dictionary_corrections: bool,
    pub snippets: bool,
    pub contexts: bool,
    pub history: bool,
    pub settings: bool,
    pub stats: bool,
    /// At least one mapping referenced a parent that was not available in
    /// this batch. The puller keeps its receive cursor unchanged so the
    /// operation is retried after a later dependency batch or snapshot.
    pub deferred: bool,
    pub applied: usize,
    pub skipped: usize,
}

impl ApplySummary {
    pub fn touched_tables(&self) -> Vec<&'static str> {
        let mut tables = Vec::new();
        if self.dictionary {
            tables.push("dictionary");
        }
        if self.dictionary_corrections {
            tables.push("dictionary_corrections");
        }
        if self.snippets {
            tables.push("snippets");
        }
        if self.contexts {
            tables.push("contexts");
        }
        if self.history {
            tables.push("history");
        }
        if self.settings {
            tables.push("settings");
        }
        if self.stats {
            tables.push("stats");
        }
        tables
    }
}

/// Holds the `sync_state.applying` flag for the duration of an apply, resetting
/// it even when an apply errors out. While the flag is set the change-capture
/// triggers stay silent; the engine logs applied ops itself with the remote's
/// original stamp so peers can dedup exactly (no echo amplification).
struct ApplyingGuard<'a>(&'a Connection);

impl<'a> ApplyingGuard<'a> {
    fn new(conn: &'a Connection) -> Result<Self> {
        sync_store::set_sync_applying(conn, true)?;
        Ok(Self(conn))
    }
}

impl Drop for ApplyingGuard<'_> {
    fn drop(&mut self) {
        if let Err(err) = sync_store::set_sync_applying(self.0, false) {
            log::error!("sync: failed to clear applying flag: {err}");
        }
    }
}

/// Logs an op that originated on THIS device (local mutation follow-ups like
/// anti-entropy tombstones and context cascades) with a fresh stamp.
pub fn append_self_log(
    conn: &Connection,
    table_name: &str,
    row_uuid: &str,
    op: &str,
) -> Result<()> {
    let Some(origin) = sync_store::self_uuid(conn)? else {
        return Ok(());
    };
    let origin_seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(origin_seq), 0) + 1 FROM sync_log WHERE origin = ?1",
        params![origin],
        |r| r.get(0),
    )?;
    let ts = sync_store::now_ms();
    sync_store::append_op(
        conn,
        &SyncOp {
            table: table_name.to_string(),
            row_uuid: row_uuid.to_string(),
            op: op.to_string(),
            ts_ms: ts,
            origin: origin.clone(),
            origin_seq,
            payload: None,
        },
        ts,
        &origin,
        origin_seq,
    )
}

fn latest_stamp(
    conn: &Connection,
    table: &str,
    row_uuid: &str,
) -> Result<Option<(i64, String, i64)>> {
    sync_store::latest_op_stamp(conn, table, row_uuid)
}

fn latest_op_name(conn: &Connection, table: &str, row_uuid: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT op FROM sync_log
          WHERE table_name = ?1 AND row_uuid = ?2
          ORDER BY seq DESC LIMIT 1",
        params![table, row_uuid],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn latest_is_delete(conn: &Connection, table: &str, row_uuid: &str) -> Result<bool> {
    Ok(latest_op_name(conn, table, row_uuid)?.as_deref() == Some("delete"))
}

fn latest_dictionary_stamp(
    conn: &Connection,
    row_uuid: &str,
) -> Result<Option<(i64, String, i64)>> {
    let regular = latest_stamp(conn, "dictionary", row_uuid)?;
    let natural_key = latest_stamp(conn, NATURAL_KEY_TOMBSTONE_TABLE, row_uuid)?;
    Ok(match (regular, natural_key) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    })
}

// ---------------------------------------------------------------------------
// Send side: resolving log entries and snapshots into wire ops
// ---------------------------------------------------------------------------

fn dictionary_payload(conn: &Connection, uuid: &str) -> Result<Option<serde_json::Value>> {
    let row = conn
        .query_row(
            "SELECT term, mistake, auto_learned, correction_count, confidence_tier, last_seen_at, created_at
             FROM dictionary WHERE uuid = ?1",
            params![uuid],
            |r| {
                Ok(DictionaryRow {
                    term: r.get(0)?,
                    // Context-specific mistakes are carried by
                    // dictionary_corrections. Do not put the legacy global
                    // projection on the wire, even if a partially migrated
                    // database still has it populated.
                    mistake: None,
                    auto_learned: r.get::<_, i64>(2)? != 0,
                    correction_count: r.get(3)?,
                    confidence_tier: r.get(4)?,
                    last_seen_at: r.get(5)?,
                    created_at: r.get(6)?,
                })
            },
        )
        .optional()?;
    Ok(row.map(|row| serde_json::to_value(row).expect("serialize dictionary row")))
}

fn dictionary_correction_payload(
    conn: &Connection,
    uuid: &str,
) -> Result<Option<serde_json::Value>> {
    let row = conn
        .query_row(
            "SELECT COALESCE(ctx.uuid, ''), COALESCE(d.uuid, ''), d.term, c.mistake, c.auto_learned,
                    c.correction_count, c.confidence_tier, c.last_seen_at, c.created_at
               FROM dictionary_corrections c
               INNER JOIN contexts ctx ON ctx.id = c.context_id
               INNER JOIN dictionary d ON d.id = c.dictionary_id
              WHERE c.uuid = ?1",
            params![uuid],
            |r| {
                Ok(DictionaryCorrectionRow {
                    context_uuid: r.get(0)?,
                    dictionary_uuid: r.get(1)?,
                    dictionary_term: r.get(2)?,
                    mistake: r.get(3)?,
                    auto_learned: r.get::<_, i64>(4)? != 0,
                    correction_count: r.get(5)?,
                    confidence_tier: r.get(6)?,
                    last_seen_at: r.get(7)?,
                    created_at: r.get(8)?,
                })
            },
        )
        .optional()?;
    Ok(row.map(|row| serde_json::to_value(row).expect("serialize dictionary correction row")))
}

fn snippet_payload(conn: &Connection, uuid: &str) -> Result<Option<serde_json::Value>> {
    let row = conn
        .query_row(
            "SELECT trigger, expansion, instructions, use_count, created_at
             FROM snippets WHERE uuid = ?1",
            params![uuid],
            |r| {
                Ok(SnippetRow {
                    trigger: r.get(0)?,
                    expansion: r.get(1)?,
                    instructions: r.get(2)?,
                    use_count: r.get(3)?,
                    created_at: r.get(4)?,
                })
            },
        )
        .optional()?;
    Ok(row.map(|row| serde_json::to_value(row).expect("serialize snippet row")))
}

fn context_aggregate(conn: &Connection, uuid: &str) -> Result<Option<serde_json::Value>> {
    let Some(context_id) = conn
        .query_row(
            "SELECT id FROM contexts WHERE uuid = ?1",
            params![uuid],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let row = conn
        .query_row(
            "SELECT name, is_everywhere, icon, tone, cleanup_intensity, color, custom_instructions,
                    contextual_formatting_disabled, pinned_at, created_at, updated_at
             FROM contexts WHERE id = ?1",
            params![context_id],
            |r| {
                Ok(ContextAggregate {
                    name: r.get(0)?,
                    is_everywhere: r.get::<_, i64>(1)? != 0,
                    icon: r.get(2)?,
                    tone: r.get(3)?,
                    cleanup_intensity: r.get(4)?,
                    color: r.get(5)?,
                    custom_instructions: r.get(6)?,
                    contextual_formatting_disabled: r.get::<_, i64>(7)? != 0,
                    pinned_at: r.get(8)?,
                    created_at: r.get(9)?,
                    updated_at: r.get(10)?,
                    targets: Vec::new(),
                    websites: Vec::new(),
                    dictionary_uuids: Vec::new(),
                    dictionary_entries: Vec::new(),
                    snippet_uuids: Vec::new(),
                })
            },
        )
        .optional()?;
    let mut aggregate = match row {
        Some(aggregate) => aggregate,
        None => return Ok(None),
    };
    let mut stmt = conn.prepare(
        "SELECT executable, platform, app_name, developer
         FROM context_targets WHERE context_id = ?1 ORDER BY executable",
    )?;
    aggregate.targets = stmt
        .query_map(params![context_id], |r| {
            Ok(TargetEntry::Tagged {
                executable: r.get(0)?,
                platform: r.get(1)?,
                app_name: r.get(2)?,
                developer: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare(
        "SELECT domain FROM context_website_targets WHERE context_id = ?1 ORDER BY domain",
    )?;
    aggregate.websites = stmt
        .query_map(params![context_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare(
        "SELECT d.uuid, d.term
           FROM dictionary_contexts dc
           JOIN dictionary d ON d.id = dc.dictionary_id
          WHERE dc.context_id = ?1 AND d.uuid IS NOT NULL
          ORDER BY d.id",
    )?;
    let dictionary_entries = stmt
        .query_map(params![context_id], |r| {
            Ok(DictionaryReference {
                uuid: r.get(0)?,
                term: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    aggregate.dictionary_uuids = dictionary_entries
        .iter()
        .map(|entry| entry.uuid.clone())
        .collect();
    aggregate.dictionary_entries = dictionary_entries;
    let mut stmt = conn.prepare(
        "SELECT s.uuid FROM snippet_contexts sc JOIN snippets s ON s.id = sc.snippet_id
         WHERE sc.context_id = ?1 ORDER BY s.id",
    )?;
    aggregate.snippet_uuids = stmt
        .query_map(params![context_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(
        serde_json::to_value(aggregate).expect("serialize context aggregate"),
    ))
}

fn transcription_payload(conn: &Connection, uuid: &str) -> Result<Option<serde_json::Value>> {
    let row = conn
        .query_row(
            "SELECT t.raw_text, t.clean_text, t.words, t.spoken_words, t.duration_ms, t.api_used,
                    t.app_name, c.uuid, t.created_at
             FROM transcriptions t LEFT JOIN contexts c ON c.id = t.context_id
             WHERE t.uuid = ?1",
            params![uuid],
            |r| {
                Ok(TranscriptionRow {
                    raw_text: r.get(0)?,
                    clean_text: r.get(1)?,
                    words: r.get(2)?,
                    spoken_words: r.get(3)?,
                    duration_ms: r.get(4)?,
                    api_used: r.get(5)?,
                    app_name: r.get(6)?,
                    context_uuid: r.get(7)?,
                    created_at: r.get(8)?,
                })
            },
        )
        .optional()?;
    Ok(row.map(|row| serde_json::to_value(row).expect("serialize transcription row")))
}

fn api_call_payload(conn: &Connection, uuid: &str) -> Result<Option<serde_json::Value>> {
    let row = conn
        .query_row(
            "SELECT t.uuid, a.model, a.provider, a.task, a.audio_ms, a.input_chars, a.output_chars, a.created_at
             FROM api_calls a LEFT JOIN transcriptions t ON t.id = a.transcription_id
             WHERE a.uuid = ?1",
            params![uuid],
            |r| {
                Ok(ApiCallRow {
                    transcription_uuid: r.get(0)?,
                    model: r.get(1)?,
                    provider: r.get(2)?,
                    task: r.get(3)?,
                    audio_ms: r.get(4)?,
                    input_chars: r.get(5)?,
                    output_chars: r.get(6)?,
                    created_at: r.get(7)?,
                })
            },
        )
        .optional()?;
    Ok(row.map(|row| serde_json::to_value(row).expect("serialize api call row")))
}

/// Resolves a collapsed log entry into a full wire op. History is append-only
/// for sync purposes: retention pruning is device-local, so old transcription
/// delete tombstones from pre-v22 databases must never be sent to peers.
fn resolve_entry(conn: &Connection, entry: &sync_store::LogEntry) -> Result<Option<SyncOp>> {
    if entry.table_name == "transcriptions" && entry.op == "delete" {
        return Ok(None);
    }
    let payload = match entry.table_name.as_str() {
        "dictionary" if entry.op == "upsert" => dictionary_payload(conn, &entry.row_uuid)?,
        "dictionary_corrections" if entry.op == "upsert" => {
            dictionary_correction_payload(conn, &entry.row_uuid)?
        }
        "snippets" if entry.op == "upsert" => snippet_payload(conn, &entry.row_uuid)?,
        "contexts" if entry.op == "upsert" => context_aggregate(conn, &entry.row_uuid)?,
        "transcriptions" if entry.op == "upsert" => transcription_payload(conn, &entry.row_uuid)?,
        "api_calls" if entry.op == "upsert" => api_call_payload(conn, &entry.row_uuid)?,
        _ => None,
    };
    let op = if entry.op == "upsert" && payload.is_none() {
        if entry.table_name == "transcriptions" {
            return Ok(None);
        }
        "delete"
    } else {
        &entry.op
    };
    Ok(Some(SyncOp {
        table: entry.table_name.clone(),
        row_uuid: entry.row_uuid.clone(),
        op: op.to_string(),
        ts_ms: entry.ts_ms,
        origin: entry.origin.clone(),
        origin_seq: entry.origin_seq,
        payload,
    }))
}

/// Collects the next batch of ops to send. `snapshot` sends the full current
/// state (new pairing or a peer whose cursor fell out of our retained log);
/// otherwise it sends the collapsed changes after `since_seq`. Returns the ops
/// plus the cursor to report: 0 while more batches follow, else the final log
/// position.
pub fn collect_ops(
    conn: &Connection,
    since_seq: i64,
    snapshot: bool,
    limit: usize,
    progress: &mut SnapshotProgress,
) -> Result<(Vec<SyncOp>, i64, bool)> {
    let limit = limit.max(1) as i64;
    if snapshot {
        // Full state for a new/rejoining peer. Every table is keyset-paginated
        // so the wire batch and sender memory stay bounded on big libraries.
        let mut ops = Vec::new();
        let now = sync_store::now_ms();
        let origin = sync_store::self_uuid(conn)?.unwrap_or_default();
        let mut origin_seq = match progress.origin_seq {
            Some(seq) => seq,
            None => {
                let seq: i64 = conn.query_row(
                    "SELECT COALESCE(MAX(origin_seq), 0) FROM sync_log WHERE origin = ?1",
                    params![origin],
                    |r| r.get(0),
                )?;
                progress.origin_seq = Some(seq);
                seq
            }
        };
        let mut push = |table: &str,
                        uuid: String,
                        payload: Option<serde_json::Value>,
                        stamp: Option<(i64, String, i64)>,
                        ops: &mut Vec<SyncOp>| {
            let (ts_ms, op_origin, op_seq) = stamp.unwrap_or_else(|| {
                origin_seq += 1;
                (now, origin.clone(), origin_seq)
            });
            if op_origin == origin {
                origin_seq = origin_seq.max(op_seq);
            }
            ops.push(SyncOp {
                table: table.to_string(),
                row_uuid: uuid,
                op: "upsert".to_string(),
                ts_ms,
                origin: op_origin,
                origin_seq: op_seq,
                payload,
            });
        };

        while (ops.len() as i64) < limit && progress.stage <= 6 {
            let capacity = limit - ops.len() as i64;
            let chunk = capacity.min(SNAPSHOT_ROW_CHUNK);
            let stage = progress.stage;
            let table = match stage {
                0 => "dictionary",
                1 => "snippets",
                2 => "contexts",
                3 => "dictionary_corrections",
                4 => "transcriptions",
                5 => "api_calls",
                6 => "tombstones",
                _ => unreachable!("snapshot stage is complete"),
            };
            if stage == 6 {
                let mut stmt = conn.prepare(
                    "SELECT seq, table_name, row_uuid, ts_ms, origin, origin_seq
                     FROM sync_log AS current
                     WHERE current.op = 'delete'
                       AND current.seq > ?1
                       AND current.seq = (
                         SELECT MAX(previous.seq) FROM sync_log AS previous
                         WHERE previous.table_name = current.table_name
                           AND previous.row_uuid = current.row_uuid
                       )
                     ORDER BY current.seq ASC LIMIT ?2",
                )?;
                let rows = stmt
                    .query_map(params![progress.last_id, chunk], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            SyncOp {
                                table: r.get(1)?,
                                row_uuid: r.get(2)?,
                                op: "delete".to_string(),
                                ts_ms: r.get(3)?,
                                origin: r.get(4)?,
                                origin_seq: r.get(5)?,
                                payload: None,
                            },
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<(i64, SyncOp)>>>()?;
                let fetched = rows.len() as i64;
                if let Some((last_seq, _)) = rows.last() {
                    progress.last_id = *last_seq;
                }
                ops.extend(rows.into_iter().map(|(_, op)| op));
                if fetched < chunk {
                    progress.stage = 6;
                    progress.origin_seq = Some(origin_seq);
                    let cursor = sync_store::max_log_seq(conn)?;
                    return Ok((ops, cursor, true));
                }
                progress.origin_seq = Some(origin_seq);
                return Ok((ops, 0, false));
            }
            // A correction row created on an incompletely migrated database
            // can temporarily have a NULL UUID. It cannot be represented as a
            // stable sync row, so omit it until the database's UUID backfill
            // repairs it. All current writes and healthy migrations provide a
            // UUID.
            let row_query = if table == "dictionary_corrections" {
                format!(
                    "SELECT id, uuid FROM {table}
                      WHERE id > ?1 AND uuid IS NOT NULL
                      ORDER BY id LIMIT ?2"
                )
            } else {
                format!("SELECT id, uuid FROM {table} WHERE id > ?1 ORDER BY id LIMIT ?2")
            };
            let mut stmt = conn.prepare(&row_query)?;
            let rows = stmt
                .query_map(params![progress.last_id, chunk], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let fetched = rows.len() as i64;
            for (id, uuid) in rows {
                progress.last_id = id;
                let payload = match table {
                    "dictionary" => dictionary_payload(conn, &uuid)?,
                    "dictionary_corrections" => dictionary_correction_payload(conn, &uuid)?,
                    "snippets" => snippet_payload(conn, &uuid)?,
                    "contexts" => context_aggregate(conn, &uuid)?,
                    "transcriptions" => transcription_payload(conn, &uuid)?,
                    "api_calls" => api_call_payload(conn, &uuid)?,
                    _ => unreachable!("snapshot table is complete"),
                };
                let stamp = latest_stamp(conn, table, &uuid)?;
                push(table, uuid, payload, stamp, &mut ops);
            }
            if fetched < chunk {
                // This table is exhausted; move to the next stage.
                progress.stage += 1;
                progress.last_id = 0;
                if progress.stage > 6 {
                    progress.origin_seq = Some(origin_seq);
                    let cursor = sync_store::max_log_seq(conn)?;
                    return Ok((ops, cursor, true));
                }
            } else {
                // Batch full; more of this table remains.
                progress.origin_seq = Some(origin_seq);
                return Ok((ops, 0, false));
            }
        }
        if progress.stage <= 6 {
            // Capacity exhausted mid-stream.
            progress.origin_seq = Some(origin_seq);
            return Ok((ops, 0, false));
        }
        progress.origin_seq = Some(origin_seq);
        let cursor = sync_store::max_log_seq(conn)?;
        return Ok((ops, cursor, true));
    }

    let entries = sync_store::changes_since(conn, since_seq, limit)?;
    let mut ops = Vec::with_capacity(entries.len());
    for entry in &entries {
        if let Some(op) = resolve_entry(conn, entry)? {
            ops.push(op);
        }
    }
    let done = entries.len() < limit as usize;
    let cursor = if done {
        sync_store::max_log_seq(conn)?
    } else {
        entries.last().map(|entry| entry.seq).unwrap_or(since_seq)
    };
    Ok((ops, cursor, done))
}

// ---------------------------------------------------------------------------
// Apply side
// ---------------------------------------------------------------------------

/// Result of one incoming op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Applied {
    Yes,
    Skipped,
    Deferred,
}

/// A pull is normally already ordered by the sender, but delta log
/// compaction and snapshot pagination mean the receiver must not rely on that
/// incidental order. Parent deletes run first so a stale child upsert cannot
/// recreate a deleted Context or canonical dictionary row. Parent upserts run
/// before correction mappings, whose payloads reference both parents by UUID.
fn apply_rank(op: &SyncOp) -> u8 {
    match (op.table.as_str(), op.is_delete()) {
        ("contexts", true)
        | ("dictionary", true)
        | ("snippets", true) => 0,
        ("dictionary", false) => 10,
        // Apply a canonical upsert first so its natural-key replacement can
        // capture/reparent children before an anti-entropy tombstone removes
        // the losing UUID. If the upsert is absent, this still behaves as a
        // normal deferred loser cleanup.
        (NATURAL_KEY_TOMBSTONE_TABLE, true) => 15,
        ("snippets", false) => 11,
        ("contexts", false) => 20,
        ("dictionary_corrections", _) => 30,
        ("transcriptions", false) => 40,
        ("api_calls", false) => 41,
        ("transcriptions", true) | ("api_calls", true) => 50,
        _ => 60,
    }
}

/// Applies a batch of remote ops inside one applying-guard window. Idempotent:
/// re-applying an already-known op is a no-op, so retries and duplicate
/// deliveries never create duplicate rows.
pub fn apply_ops(conn: &Connection, ops: &[SyncOp]) -> Result<ApplySummary> {
    let _guard = ApplyingGuard::new(conn)?;
    let mut summary = ApplySummary::default();
    // Apply in dependency order rather than trusting the order in a delta
    // batch. Snapshot batches are naturally ordered too, but this explicit
    // sort makes hand-built batches and compacted logs safe as well.
    let mut ordered: Vec<(usize, &SyncOp)> = ops.iter().enumerate().collect();
    ordered.sort_by_key(|(index, op)| (apply_rank(op), *index));
    for (_, op) in ordered {
        match op.table.as_str() {
            "dictionary" | NATURAL_KEY_TOMBSTONE_TABLE => {
                if apply_dictionary_op(conn, op)? == Applied::Yes {
                    summary.dictionary = true;
                    summary.applied += 1;
                } else {
                    summary.skipped += 1;
                }
            }
            "snippets" => {
                if apply_snippet_op(conn, op)? == Applied::Yes {
                    summary.snippets = true;
                    summary.applied += 1;
                } else {
                    summary.skipped += 1;
                }
            }
            "contexts" => {
                if apply_context_op(conn, op)? == Applied::Yes {
                    summary.contexts = true;
                    summary.applied += 1;
                } else {
                    summary.skipped += 1;
                }
            }
            "dictionary_corrections" => {
                match apply_dictionary_correction_op(conn, op)? {
                    Applied::Yes => {
                        // Keep the legacy dictionary refresh bit set as well
                        // as a precise flag. Existing desktop clients listen
                        // only for "dictionary"; Context-aware clients can
                        // use the more specific table name.
                        summary.dictionary = true;
                        summary.dictionary_corrections = true;
                        summary.applied += 1;
                    }
                    Applied::Skipped => summary.skipped += 1,
                    Applied::Deferred => {
                        summary.deferred = true;
                        summary.skipped += 1;
                    }
                }
            }
            "transcriptions" => {
                if apply_transcription_op(conn, op)? == Applied::Yes {
                    summary.history = true;
                    summary.applied += 1;
                } else {
                    summary.skipped += 1;
                }
            }
            "api_calls" => {
                if apply_api_call_op(conn, op)? == Applied::Yes {
                    summary.history = true;
                    summary.applied += 1;
                } else {
                    summary.skipped += 1;
                }
            }
            other => {
                log::warn!("sync: ignoring op for unknown table {other:?}");
                summary.skipped += 1;
            }
        }
    }
    Ok(summary)
}

/// Logs a successfully applied remote op with its original stamp, so peers
/// that pull from us see the same op with the same identity (dedup works) and
/// our own LWW comparisons stay consistent.
fn log_applied(conn: &Connection, op: &SyncOp) -> Result<()> {
    sync_store::append_op(conn, op, op.ts_ms, &op.origin, op.origin_seq)
}

/// When a local row wins a natural-key collision, the remote's losing row must
/// eventually vanish everywhere. Log an anti-entropy tombstone for it.
fn log_anti_entropy_delete(conn: &Connection, table: &str, row_uuid: &str) -> Result<()> {
    append_self_log(conn, table, row_uuid, "delete")
}

fn log_natural_key_delete(conn: &Connection, row_uuid: &str) -> Result<()> {
    // `table_name` is intentionally a sync-log namespace, not a SQLite
    // table. The schema only permits upsert/delete, so the namespace carries
    // the distinction needed by child natural-key fallback.
    append_self_log(conn, NATURAL_KEY_TOMBSTONE_TABLE, row_uuid, "delete")
}

/// Contexts whose junction rows referenced a dictionary/snippet row that is
/// about to be hard-deleted. Their aggregates change, so they are re-logged
/// (the cascade delete of the junction rows is trigger-suppressed here).
fn log_contexts_referencing_dictionary(conn: &Connection, dictionary_uuid: &str) -> Result<()> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare(
            "SELECT dc.context_id FROM dictionary_contexts dc
             JOIN dictionary d ON d.id = dc.dictionary_id WHERE d.uuid = ?1",
        )?;
        let collected = stmt
            .query_map(params![dictionary_uuid], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        collected
    };
    log_context_upserts(conn, &ids)
}

fn log_contexts_referencing_snippet(conn: &Connection, snippet_uuid: &str) -> Result<()> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare(
            "SELECT sc.context_id FROM snippet_contexts sc
             JOIN snippets s ON s.id = sc.snippet_id WHERE s.uuid = ?1",
        )?;
        let collected = stmt
            .query_map(params![snippet_uuid], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        collected
    };
    log_context_upserts(conn, &ids)
}

fn log_context_upserts(conn: &Connection, context_ids: &[i64]) -> Result<()> {
    for context_id in context_ids {
        let uuid: Option<String> = conn
            .query_row(
                "SELECT uuid FROM contexts WHERE id = ?1",
                params![context_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(uuid) = uuid {
            append_self_log(conn, "contexts", &uuid, "upsert")?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct DictionaryCorrectionSnapshot {
    /// `None` is possible only on a partially migrated database. Such a row
    /// receives a fresh UUID if it can be safely restored.
    uuid: Option<String>,
    context_id: i64,
    mistake: String,
    auto_learned: bool,
    correction_count: i64,
    confidence_tier: String,
    last_seen_at: Option<String>,
    created_at: String,
}

#[derive(Debug, Default)]
struct DictionaryChildrenSnapshot {
    context_ids: Vec<i64>,
    corrections: Vec<DictionaryCorrectionSnapshot>,
}

fn capture_dictionary_children(
    conn: &Connection,
    dictionary_uuid: &str,
) -> Result<DictionaryChildrenSnapshot> {
    let Some(dictionary_id) = conn
        .query_row(
            "SELECT id FROM dictionary WHERE uuid = ?1",
            params![dictionary_uuid],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(DictionaryChildrenSnapshot::default());
    };

    let mut context_ids: Vec<i64> = conn
        .prepare(
            "SELECT context_id FROM dictionary_contexts
              WHERE dictionary_id = ?1 ORDER BY context_id",
        )?
        .query_map(params![dictionary_id], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let corrections = conn
        .prepare(
            "SELECT uuid, context_id, mistake, auto_learned, correction_count,
                    confidence_tier, last_seen_at, created_at
               FROM dictionary_corrections
              WHERE dictionary_id = ?1 ORDER BY id",
        )?
        .query_map(params![dictionary_id], |r| {
            Ok(DictionaryCorrectionSnapshot {
                uuid: r.get(0)?,
                context_id: r.get(1)?,
                mistake: r.get(2)?,
                auto_learned: r.get::<_, i64>(3)? != 0,
                correction_count: r.get(4)?,
                confidence_tier: r.get(5)?,
                last_seen_at: r.get(6)?,
                created_at: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // A malformed or partially migrated database may have a mapping without
    // its junction row. Preserve the mapping's Context too, then let the
    // restore path repair the missing assignment.
    for correction in &corrections {
        if !context_ids.contains(&correction.context_id) {
            context_ids.push(correction.context_id);
        }
    }

    Ok(DictionaryChildrenSnapshot {
        context_ids,
        corrections,
    })
}

fn correction_confidence_rank(tier: &str) -> u8 {
    match tier {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn later_timestamp(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

/// Reattaches the children of a locally losing canonical dictionary row to
/// the remote winner. The canonical term is globally unique, so keeping the
/// shared row while dropping all of its Context mappings would silently lose
/// user vocabulary. Mapping natural-key conflicts are handled conservatively:
/// a pre-existing mapping for another canonical term wins, while identical
/// mappings merge their safe metadata.
fn restore_dictionary_children(
    conn: &Connection,
    children: &DictionaryChildrenSnapshot,
    winner_dictionary_id: i64,
) -> Result<()> {
    let mut touched_contexts = Vec::new();
    for context_id in &children.context_ids {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM contexts WHERE id = ?1)",
            params![context_id],
            |r| r.get(0),
        )?;
        if !exists {
            continue;
        }
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
             VALUES (?1, ?2)",
            params![context_id, winner_dictionary_id],
        )?;
        if inserted > 0 {
            touched_contexts.push(*context_id);
        }
    }

    for source in &children.corrections {
        let context_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM contexts WHERE id = ?1)",
            params![source.context_id],
            |r| r.get(0),
        )?;
        if !context_exists {
            // The source Context was deleted independently. Do not recreate
            // it from a child row whose parent no longer exists.
            if let Some(uuid) = source.uuid.as_deref() {
                log_anti_entropy_delete(conn, "dictionary_corrections", uuid)?;
            }
            continue;
        }

        let other_mapping: Option<(i64, Option<String>)> = conn
            .query_row(
                "SELECT dictionary_id, uuid FROM dictionary_corrections
                  WHERE context_id = ?1 AND mistake = ?2
                    AND dictionary_id != ?3
                  ORDER BY id LIMIT 1",
                params![source.context_id, source.mistake, winner_dictionary_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if other_mapping.is_some() {
            // Two canonical terms cannot safely claim the same wrong spelling
            // in one Context. Preserve the already-effective mapping rather
            // than changing what the user will receive.
            if let Some(uuid) = source.uuid.as_deref() {
                log_anti_entropy_delete(conn, "dictionary_corrections", uuid)?;
            }
            continue;
        }

        let existing: Option<ExistingCorrection> = conn
            .query_row(
                "SELECT id, uuid, auto_learned, correction_count,
                        confidence_tier, last_seen_at
                   FROM dictionary_corrections
                  WHERE context_id = ?1 AND dictionary_id = ?2 AND mistake = ?3
                  LIMIT 1",
                params![source.context_id, winner_dictionary_id, source.mistake],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get::<_, i64>(2)? != 0,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()?;

        if let Some((target_id, target_uuid, target_auto, target_count, target_tier, target_last)) =
            existing
        {
            let target_is_manual = !target_auto;
            let source_is_manual = !source.auto_learned;
            let (auto_learned, correction_count, confidence_tier, last_seen_at) =
                if source_is_manual || target_is_manual {
                    // Manual authority wins. Retain the manual row's count and
                    // tier unless the source itself is the manual authority.
                    if source_is_manual && !target_is_manual {
                        (
                            false,
                            source.correction_count,
                            "manual".to_string(),
                            source.last_seen_at.clone(),
                        )
                    } else {
                        (
                            false,
                            target_count,
                            "manual".to_string(),
                            target_last.clone(),
                        )
                    }
                } else {
                    (
                        true,
                        target_count + source.correction_count,
                        if correction_confidence_rank(&source.confidence_tier)
                            > correction_confidence_rank(&target_tier)
                        {
                            source.confidence_tier.clone()
                        } else {
                            target_tier.clone()
                        },
                        later_timestamp(target_last.clone(), source.last_seen_at.clone()),
                    )
                };
            conn.execute(
                "UPDATE dictionary_corrections
                    SET auto_learned = ?2, correction_count = ?3,
                        confidence_tier = ?4, last_seen_at = ?5
                  WHERE id = ?1",
                params![
                    target_id,
                    auto_learned as i64,
                    correction_count,
                    confidence_tier,
                    last_seen_at
                ],
            )?;
            if let Some(target_uuid) = target_uuid.as_deref() {
                append_self_log(conn, "dictionary_corrections", target_uuid, "upsert")?;
            }
            if let Some(source_uuid) = source.uuid.as_deref() {
                if Some(source_uuid) != target_uuid.as_deref() {
                    log_anti_entropy_delete(conn, "dictionary_corrections", source_uuid)?;
                }
            }
            continue;
        }

        let mapping_uuid = source
            .uuid
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let inserted = conn.execute(
            "INSERT INTO dictionary_corrections
               (uuid, context_id, dictionary_id, mistake, auto_learned,
                correction_count, confidence_tier, last_seen_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                mapping_uuid,
                source.context_id,
                winner_dictionary_id,
                source.mistake,
                source.auto_learned as i64,
                source.correction_count,
                source.confidence_tier,
                source.last_seen_at,
                source.created_at,
            ],
        );
        match inserted {
            Ok(_) => {
                append_self_log(conn, "dictionary_corrections", &mapping_uuid, "upsert")?;
                if !touched_contexts.contains(&source.context_id) {
                    touched_contexts.push(source.context_id);
                }
            }
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                // A UUID collision is exceptionally unlikely, but a safe
                // fresh identity is preferable to overwriting an unrelated
                // mapping. The original identity is tombstoned below when it
                // came from a real row.
                let fresh_uuid = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO dictionary_corrections
                       (uuid, context_id, dictionary_id, mistake, auto_learned,
                        correction_count, confidence_tier, last_seen_at, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        fresh_uuid,
                        source.context_id,
                        winner_dictionary_id,
                        source.mistake,
                        source.auto_learned as i64,
                        source.correction_count,
                        source.confidence_tier,
                        source.last_seen_at,
                        source.created_at,
                    ],
                )?;
                append_self_log(conn, "dictionary_corrections", &fresh_uuid, "upsert")?;
                if let Some(source_uuid) = source.uuid.as_deref() {
                    log_anti_entropy_delete(conn, "dictionary_corrections", source_uuid)?;
                }
            }
            Err(err) => {
                return Err(anyhow!(
                    "sync: restoring dictionary correction failed: {err}"
                ))
            }
        }
    }
    log_context_upserts(conn, &touched_contexts)?;
    Ok(())
}

fn apply_dictionary_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.table == NATURAL_KEY_TOMBSTONE_TABLE {
        return apply_simple_delete(conn, op, "dictionary", NATURAL_KEY_TOMBSTONE_TABLE);
    }
    if op.is_delete() {
        return apply_simple_delete(conn, op, "dictionary", "dictionary");
    }
    if let Some(stamp) = latest_dictionary_stamp(conn, &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    let row: DictionaryRow = serde_json::from_value(
        op.payload
            .clone()
            .ok_or_else(|| anyhow!("dictionary upsert missing payload"))?,
    )
    .context("invalid dictionary payload")?;

    let insert = |conn: &Connection| -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO dictionary (uuid, term, mistake, auto_learned, correction_count, confidence_tier, last_seen_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(uuid) DO UPDATE SET
               term = excluded.term, mistake = NULL,
               auto_learned = excluded.auto_learned,
               correction_count = excluded.correction_count,
               confidence_tier = excluded.confidence_tier,
               last_seen_at = excluded.last_seen_at",
            params![
                op.row_uuid,
                row.term,
                Option::<String>::None,
                row.auto_learned as i64,
                row.correction_count,
                row.confidence_tier,
                row.last_seen_at,
                row.created_at
            ],
        )
    };
    apply_with_natural_key_resolution(conn, op, "dictionary", insert, &|conn| {
        conflicting_uuid(conn, "dictionary", "term", &row.term, &op.row_uuid)
    })
}

fn resolve_correction_dictionary_id(
    conn: &Connection,
    row: &DictionaryCorrectionRow,
) -> Result<Option<i64>> {
    let by_uuid = conn
        .query_row(
            "SELECT id FROM dictionary WHERE uuid = ?1",
            params![row.dictionary_uuid],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    if by_uuid.is_some() {
        return Ok(by_uuid);
    }

    // A canonical UUID can legitimately differ on two devices when both
    // created the same term offline. The dictionary conflict path keeps the
    // higher-stamped canonical row and re-parents its children; this fallback
    // lets a child op that was already in flight follow that natural key too.
    let term = row.dictionary_term.trim();
    if term.is_empty() {
        return Ok(None);
    }
    conn.query_row(
        "SELECT id FROM dictionary WHERE term = ?1",
        params![term],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Runs a parent/child replacement under a SQLite savepoint. Applying a
/// remote batch is intentionally not one large transaction because some
/// existing helpers open their own transaction, but a canonical natural-key
/// replacement must be all-or-nothing or a failed child restore would lose
/// the loser's mappings permanently.
fn with_sync_savepoint<T>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    conn.execute_batch("SAVEPOINT sync_dictionary_reparent")?;
    match f(conn) {
        Ok(value) => {
            conn.execute_batch("RELEASE sync_dictionary_reparent")?;
            Ok(value)
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch("ROLLBACK TO sync_dictionary_reparent")
            {
                log::error!(
                    "sync: failed to roll back dictionary reparent savepoint: {rollback_error}"
                );
            }
            if let Err(release_error) = conn.execute_batch("RELEASE sync_dictionary_reparent") {
                log::error!(
                    "sync: failed to release dictionary reparent savepoint: {release_error}"
                );
            }
            Err(error)
        }
    }
}

fn correction_target_for_natural_key(
    conn: &Connection,
    context_id: i64,
    mistake: &str,
    exclude_id: Option<i64>,
) -> Result<Option<CorrectionTarget>> {
    conn.query_row(
        "SELECT id, uuid, dictionary_id, auto_learned
           FROM dictionary_corrections
          WHERE context_id = ?1 AND mistake = ?2
            AND (?3 IS NULL OR id != ?3)
          ORDER BY id LIMIT 1",
        params![context_id, mistake, exclude_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0)),
    )
    .optional()
    .map_err(Into::into)
}

fn correction_uuid_exists(conn: &Connection, uuid: &str) -> Result<Option<CorrectionIdentity>> {
    conn.query_row(
        "SELECT id, context_id, dictionary_id, mistake, auto_learned
           FROM dictionary_corrections WHERE uuid = ?1",
        params![uuid],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get::<_, i64>(4)? != 0,
            ))
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Manual mappings have priority over automatic mappings regardless of the
/// arrival order or wall-clock stamp. Two mappings with the same authority
/// use the normal total LWW stamp.
fn remote_correction_wins(
    remote_auto_learned: bool,
    local_auto_learned: bool,
    remote_stamp: (i64, &str, i64),
    local_stamp: (i64, String, i64),
) -> bool {
    match (remote_auto_learned, local_auto_learned) {
        (false, true) => true,
        (true, false) => false,
        _ => remote_stamp > (local_stamp.0, local_stamp.1.as_str(), local_stamp.2),
    }
}

fn delete_local_correction_for_remote_winner(
    conn: &Connection,
    correction_id: i64,
    correction_uuid: Option<&str>,
) -> Result<()> {
    conn.execute(
        "DELETE FROM dictionary_corrections WHERE id = ?1",
        params![correction_id],
    )?;
    if let Some(uuid) = correction_uuid {
        append_self_log(conn, "dictionary_corrections", uuid, "delete")?;
    }
    Ok(())
}

fn apply_dictionary_correction_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() {
        if let Some(stamp) = latest_stamp(conn, "dictionary_corrections", &op.row_uuid)? {
            if !op.newer_than(&stamp) {
                return Ok(Applied::Skipped);
            }
        }
        let context_id: Option<i64> = conn
            .query_row(
                "SELECT context_id FROM dictionary_corrections WHERE uuid = ?1",
                params![op.row_uuid],
                |r| r.get(0),
            )
            .optional()?;
        let deleted = conn.execute(
            "DELETE FROM dictionary_corrections WHERE uuid = ?1",
            params![op.row_uuid],
        )?;
        log_applied(conn, op)?;
        if deleted > 0 {
            if let Some(context_id) = context_id {
                log_context_upserts(conn, &[context_id])?;
            }
        }
        return Ok(Applied::Yes);
    }

    if let Some(stamp) = latest_stamp(conn, "dictionary_corrections", &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    let row: DictionaryCorrectionRow = serde_json::from_value(
        op.payload
            .clone()
            .ok_or_else(|| anyhow!("dictionary correction upsert missing payload"))?,
    )
    .context("invalid dictionary correction payload")?;

    let Some(context_id) = conn
        .query_row(
            "SELECT id FROM contexts WHERE uuid = ?1",
            params![row.context_uuid],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        // Never recreate a deleted Context from a child mapping. If a
        // tombstone proves that the Context was deliberately removed, make
        // the child tombstone durable too; otherwise retain the operation for
        // a later batch in which the parent may arrive.
        if latest_is_delete(conn, "contexts", &row.context_uuid)? {
            log_anti_entropy_delete(conn, "dictionary_corrections", &op.row_uuid)?;
            return Ok(Applied::Skipped);
        }
        return Ok(Applied::Deferred);
    };
    let dictionary_is_deleted = latest_is_delete(conn, "dictionary", &row.dictionary_uuid)?;
    if dictionary_is_deleted {
        // Do not use the term fallback after an explicit canonical delete. A
        // later row with the same term may be a new canonical identity, and
        // attaching that stale child would resurrect deleted vocabulary. A
        // natural-key loser uses the separate sync-log namespace below and is
        // intentionally allowed to follow the surviving canonical row.
        log_anti_entropy_delete(conn, "dictionary_corrections", &op.row_uuid)?;
        return Ok(Applied::Skipped);
    }
    let Some(dictionary_id) = resolve_correction_dictionary_id(conn, &row)? else {
        // The sender's canonical row may be in a later batch. Never
        // manufacture a canonical term or attach the mapping to Everywhere;
        // the puller will retry this batch after the dependency arrives.
        return Ok(Applied::Deferred);
    };

    let existing_by_uuid = correction_uuid_exists(conn, &op.row_uuid)?;
    if let Some((
        existing_id,
        _existing_context_id,
        _existing_dictionary_id,
        _existing_mistake,
        existing_auto,
    )) = existing_by_uuid.as_ref()
    {
        // A persistent mapping UUID is allowed to move when Context deletion
        // moves it to Everywhere. Treat that as an update, but protect a
        // manual row from an automatic remote overwrite.
        if !*existing_auto && row.auto_learned {
            append_self_log(conn, "dictionary_corrections", &op.row_uuid, "upsert")?;
            return Ok(Applied::Skipped);
        }
        if let Some(conflict) =
            correction_target_for_natural_key(conn, context_id, &row.mistake, Some(*existing_id))?
        {
            let conflict_stamp = latest_stamp(
                conn,
                "dictionary_corrections",
                conflict.1.as_deref().unwrap_or_default(),
            )?
            .unwrap_or((0, String::new(), 0));
            if !remote_correction_wins(row.auto_learned, conflict.3, op.stamp(), conflict_stamp) {
                append_self_log(conn, "dictionary_corrections", &op.row_uuid, "upsert")?;
                return Ok(Applied::Skipped);
            }
            delete_local_correction_for_remote_winner(conn, conflict.0, conflict.1.as_deref())?;
        }
        conn.execute(
            "UPDATE dictionary_corrections
                SET context_id = ?2, dictionary_id = ?3, mistake = ?4,
                    auto_learned = ?5, correction_count = ?6,
                    confidence_tier = ?7, last_seen_at = ?8, created_at = ?9
              WHERE id = ?1",
            params![
                existing_id,
                context_id,
                dictionary_id,
                row.mistake,
                row.auto_learned as i64,
                row.correction_count,
                row.confidence_tier,
                row.last_seen_at,
                row.created_at,
            ],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
             VALUES (?1, ?2)",
            params![context_id, dictionary_id],
        )?;
        // The child mapping is the only authoritative correction projection.
        // Clear a stale legacy value if this row came from a partially
        // migrated database or an older local write.
        conn.execute(
            "UPDATE dictionary SET mistake = NULL WHERE id = ?1",
            params![dictionary_id],
        )?;
        log_applied(conn, op)?;
        log_context_upserts(conn, &[context_id])?;
        return Ok(Applied::Yes);
    }

    if let Some(conflict) = correction_target_for_natural_key(conn, context_id, &row.mistake, None)?
    {
        let conflict_stamp = conflict
            .1
            .as_deref()
            .map(|uuid| latest_stamp(conn, "dictionary_corrections", uuid))
            .transpose()?
            .flatten()
            .unwrap_or((0, String::new(), 0));
        if !remote_correction_wins(row.auto_learned, conflict.3, op.stamp(), conflict_stamp) {
            // The remote mapping is the loser. Keep the local row and make
            // the decision durable on the sender through a tombstone for the
            // losing UUID.
            log_anti_entropy_delete(conn, "dictionary_corrections", &op.row_uuid)?;
            return Ok(Applied::Skipped);
        }
        delete_local_correction_for_remote_winner(conn, conflict.0, conflict.1.as_deref())?;
    }

    // A correction implies that its canonical item is assigned to the same
    // Context. This repairs a batch where the independent child delta arrives
    // after a Context aggregate that did not yet contain the membership.
    conn.execute(
        "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id)
         VALUES (?1, ?2)",
        params![context_id, dictionary_id],
    )?;
    conn.execute(
        "UPDATE dictionary SET mistake = NULL WHERE id = ?1",
        params![dictionary_id],
    )?;
    conn.execute(
        "INSERT INTO dictionary_corrections
           (uuid, context_id, dictionary_id, mistake, auto_learned,
            correction_count, confidence_tier, last_seen_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            op.row_uuid,
            context_id,
            dictionary_id,
            row.mistake,
            row.auto_learned as i64,
            row.correction_count,
            row.confidence_tier,
            row.last_seen_at,
            row.created_at,
        ],
    )?;
    log_applied(conn, op)?;
    log_context_upserts(conn, &[context_id])?;
    Ok(Applied::Yes)
}

fn apply_snippet_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() {
        let result = apply_simple_delete(conn, op, "snippets", "snippets");
        if matches!(result, Ok(Applied::Yes)) {
            db::invalidate_snippet_cache();
        }
        return result;
    }
    if let Some(stamp) = latest_stamp(conn, "snippets", &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    let row: SnippetRow = serde_json::from_value(
        op.payload
            .clone()
            .ok_or_else(|| anyhow!("snippet upsert missing payload"))?,
    )
    .context("invalid snippet payload")?;

    let insert = |conn: &Connection| -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO snippets (uuid, trigger, expansion, instructions, use_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(uuid) DO UPDATE SET
               trigger = excluded.trigger, expansion = excluded.expansion,
               instructions = excluded.instructions, use_count = excluded.use_count",
            params![
                op.row_uuid,
                row.trigger,
                row.expansion,
                row.instructions,
                row.use_count,
                row.created_at
            ],
        )
    };
    let result = apply_with_natural_key_resolution(conn, op, "snippets", insert, &|conn| {
        conflicting_uuid(conn, "snippets", "trigger", &row.trigger, &op.row_uuid)
    });
    if matches!(result, Ok(Applied::Yes)) {
        db::invalidate_snippet_cache();
    }
    result
}

/// Shared upsert flow with natural-key collision resolution. `insert` writes
/// the row (upsert by uuid); `find_conflict` returns the uuid of a DIFFERENT
/// local row holding the same natural key. Context membership cascades are
/// logged before deleting dictionary/snippet losers.
fn apply_with_natural_key_resolution(
    conn: &Connection,
    op: &SyncOp,
    table: &str,
    insert: impl Fn(&Connection) -> rusqlite::Result<usize>,
    find_conflict: &dyn Fn(&Connection) -> Result<Option<String>>,
) -> Result<Applied> {
    match insert(conn) {
        Ok(_) => {
            log_applied(conn, op)?;
            Ok(Applied::Yes)
        }
        Err(rusqlite::Error::SqliteFailure(err, _))
            if err.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            // Another local row owns the same natural key. Deterministic
            // winner: the higher stamp.
            let Some(conflict_uuid) = find_conflict(conn)? else {
                return Err(anyhow!(
                    "sync: {table} constraint failed but no conflicting row found for {}",
                    op.row_uuid
                ));
            };
            let conflict_stamp =
                latest_stamp(conn, table, &conflict_uuid)?.unwrap_or((0, String::new(), 0));
            if op.newer_than(&conflict_stamp) {
                // Remote row wins: remove the local loser (with its cascades)
                // and retry the insert.
                if table == "dictionary" {
                    // Capture before the parent delete, because the foreign
                    // key cascade would otherwise erase the mappings and
                    // memberships attached to the losing canonical UUID.
                    let dictionary_children = capture_dictionary_children(conn, &conflict_uuid)?;
                    return with_sync_savepoint(conn, |conn| {
                        log_contexts_referencing_dictionary(conn, &conflict_uuid)?;
                        conn.execute(
                            "DELETE FROM dictionary WHERE uuid = ?1",
                            params![conflict_uuid],
                        )?;
                        insert(conn).map_err(|e| anyhow!("sync: retry insert failed: {e}"))?;
                        let winner_id: i64 = conn.query_row(
                            "SELECT id FROM dictionary WHERE uuid = ?1",
                            params![op.row_uuid],
                            |r| r.get(0),
                        )?;
                        restore_dictionary_children(conn, &dictionary_children, winner_id)?;
                        // Retain a durable tombstone for the local loser so
                        // peers that have not seen the winning UUID cannot
                        // resurrect the old canonical row later.
                        log_natural_key_delete(conn, &conflict_uuid)?;
                        log_applied(conn, op)?;
                        Ok(Applied::Yes)
                    });
                }

                if table == "snippets" {
                    log_contexts_referencing_snippet(conn, &conflict_uuid)?;
                }
                conn.execute(
                    &format!("DELETE FROM {table} WHERE uuid = ?1"),
                    params![conflict_uuid],
                )?;
                insert(conn).map_err(|e| anyhow!("sync: retry insert failed: {e}"))?;
                log_applied(conn, op)?;
                Ok(Applied::Yes)
            } else {
                // Local row wins: tell the peer (eventually) to drop its loser.
                if table == "dictionary" {
                    log_natural_key_delete(conn, &op.row_uuid)?;
                } else {
                    log_anti_entropy_delete(conn, table, &op.row_uuid)?;
                }
                Ok(Applied::Skipped)
            }
        }
        Err(err) => Err(anyhow!("sync: {table} upsert failed: {err}")),
    }
}

/// Finds a row with the same natural key but a different uuid, if any.
fn conflicting_uuid(
    conn: &Connection,
    table: &str,
    column: &str,
    value: &str,
    exclude_uuid: &str,
) -> Result<Option<String>> {
    let collate = if table == "contexts" {
        "COLLATE NOCASE"
    } else {
        ""
    };
    let context_scope = if table == "contexts" {
        " AND is_everywhere = 0"
    } else {
        ""
    };
    let uuid: Option<String> = conn
        .query_row(
            &format!(
                "SELECT uuid FROM {table} WHERE {column} = ?1 {collate}{context_scope} AND uuid != ?2 LIMIT 1"
            ),
            params![value, exclude_uuid],
            |r| r.get(0),
        )
        .optional()?;
    Ok(uuid)
}

fn apply_simple_delete(
    conn: &Connection,
    op: &SyncOp,
    table: &str,
    log_table: &str,
) -> Result<Applied> {
    if let Some(stamp) = latest_stamp(conn, log_table, &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    // Capture cascades before the delete (the trigger-suppressed cascade would
    // otherwise never be logged).
    if table == "dictionary" {
        log_contexts_referencing_dictionary(conn, &op.row_uuid)?;
    } else if table == "snippets" {
        log_contexts_referencing_snippet(conn, &op.row_uuid)?;
    }
    let deleted = conn.execute(
        &format!("DELETE FROM {table} WHERE uuid = ?1"),
        params![op.row_uuid],
    )?;
    log_applied(conn, op)?;
    if deleted == 0 {
        log::debug!("sync: delete for absent {} row {}", table, op.row_uuid);
    }
    Ok(Applied::Yes)
}

fn apply_context_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() {
        return apply_context_delete(conn, op);
    }
    if let Some(stamp) = latest_stamp(conn, "contexts", &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    let aggregate: ContextAggregate = serde_json::from_value(
        op.payload
            .clone()
            .ok_or_else(|| anyhow!("context upsert missing payload"))?,
    )
    .context("invalid context payload")?;

    if aggregate.is_everywhere {
        apply_everywhere_aggregate(conn, &aggregate)?;
        log_applied(conn, op)?;
        return Ok(Applied::Yes);
    }

    let insert = |conn: &Connection| -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO contexts (uuid, name, is_everywhere, icon, tone, cleanup_intensity, color,
                                   custom_instructions, contextual_formatting_disabled, pinned_at,
                                   created_at, updated_at)
             VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(uuid) DO UPDATE SET
               name = excluded.name, icon = excluded.icon, tone = excluded.tone,
               cleanup_intensity = excluded.cleanup_intensity, color = excluded.color,
               custom_instructions = excluded.custom_instructions,
               contextual_formatting_disabled = excluded.contextual_formatting_disabled,
               pinned_at = excluded.pinned_at, updated_at = excluded.updated_at",
            params![
                op.row_uuid,
                aggregate.name,
                aggregate.icon,
                aggregate.tone,
                aggregate.cleanup_intensity,
                aggregate.color,
                aggregate.custom_instructions,
                aggregate.contextual_formatting_disabled as i64,
                aggregate.pinned_at,
                aggregate.created_at,
                aggregate.updated_at
            ],
        )
    };
    // Name conflicts resolve like any natural key, EXCEPT the loser is a
    // context: deleting it must reuse the app's delete semantics (junctions
    // move to Everywhere), not a bare DELETE.
    match insert(conn) {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(err, _))
            if err.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            let Some(conflict_uuid) =
                conflicting_uuid(conn, "contexts", "name", &aggregate.name, &op.row_uuid)?
            else {
                return Err(anyhow!(
                    "sync: context name conflict without conflicting row ({})",
                    aggregate.name
                ));
            };
            let conflict_stamp =
                latest_stamp(conn, "contexts", &conflict_uuid)?.unwrap_or((0, String::new(), 0));
            if op.newer_than(&conflict_stamp) {
                delete_context_by_uuid(conn, &conflict_uuid)?;
                insert(conn).map_err(|e| anyhow!("sync: context retry insert failed: {e}"))?;
            } else {
                log_anti_entropy_delete(conn, "contexts", &op.row_uuid)?;
                return Ok(Applied::Skipped);
            }
        }
        Err(err) => return Err(anyhow!("sync: context upsert failed: {err}")),
    }

    let context_id: i64 = conn.query_row(
        "SELECT id FROM contexts WHERE uuid = ?1",
        params![op.row_uuid],
        |r| r.get(0),
    )?;
    reconcile_context_children(conn, context_id, &aggregate)?;
    log_applied(conn, op)?;
    Ok(Applied::Yes)
}

/// Applies the Everywhere aggregate: style/name edits sync, the row itself is
/// never created or deleted, and targets are impossible by construction.
fn apply_everywhere_aggregate(conn: &Connection, aggregate: &ContextAggregate) -> Result<()> {
    let everywhere_id = db::ensure_everywhere_context_conn(conn)?;
    // A rename can still collide with a user context's name; keep the local
    // name on collision rather than deleting a user context over it.
    let rename = conn.execute(
        "UPDATE contexts SET name = ?1, updated_at = ?2 WHERE id = ?3",
        params![aggregate.name, aggregate.updated_at, everywhere_id],
    );
    if let Err(err) = rename {
        log::warn!("sync: skipping Everywhere rename: {err}");
    }
    conn.execute(
        "UPDATE contexts SET icon = ?1, tone = ?2, cleanup_intensity = ?3, color = ?4,
                custom_instructions = ?5, contextual_formatting_disabled = ?6, pinned_at = ?7
         WHERE id = ?8",
        params![
            aggregate.icon,
            aggregate.tone,
            aggregate.cleanup_intensity,
            aggregate.color,
            aggregate.custom_instructions,
            aggregate.contextual_formatting_disabled as i64,
            aggregate.pinned_at,
            everywhere_id
        ],
    )?;
    reconcile_context_members(conn, everywhere_id, aggregate)?;
    Ok(())
}

fn delete_context_by_uuid(conn: &Connection, uuid: &str) -> Result<()> {
    let Some(context_id) = conn
        .query_row(
            "SELECT id FROM contexts WHERE uuid = ?1",
            params![uuid],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(());
    };
    let is_everywhere: bool = conn.query_row(
        "SELECT is_everywhere != 0 FROM contexts WHERE id = ?1",
        params![context_id],
        |r| r.get(0),
    )?;
    if is_everywhere {
        // The built-in fallback Context is undeletable locally and must not
        // be removed by a malformed or stale remote tombstone either.
        return Ok(());
    }
    let everywhere_id = db::ensure_everywhere_context_conn(conn)?;
    let correction_uuids: Vec<String> = conn
        .prepare(
            "SELECT uuid FROM dictionary_corrections
              WHERE context_id = ?1 AND uuid IS NOT NULL ORDER BY id",
        )?
        .query_map(params![context_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Reuse the app's delete semantics: junction rows move to Everywhere so
    // vocabulary is never orphaned. Trigger-suppressed; the delete op itself
    // is logged by the caller. Because this is a remote apply, the local
    // triggers are intentionally silent; explicitly log the moved mappings
    // and the new Everywhere aggregate below so this device can relay the
    // same deletion semantics to its other peers.
    db::delete_context_conn(conn, context_id)?;
    for correction_uuid in correction_uuids {
        let still_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM dictionary_corrections WHERE uuid = ?1)",
            params![correction_uuid],
            |r| r.get(0),
        )?;
        if still_exists {
            append_self_log(conn, "dictionary_corrections", &correction_uuid, "upsert")?;
        } else {
            log_anti_entropy_delete(conn, "dictionary_corrections", &correction_uuid)?;
        }
    }
    log_context_upserts(conn, &[everywhere_id])?;
    Ok(())
}

fn apply_context_delete(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if let Some(stamp) = latest_stamp(conn, "contexts", &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    delete_context_by_uuid(conn, &op.row_uuid)?;
    log_applied(conn, op)?;
    Ok(Applied::Yes)
}

fn reconcile_context_children(
    conn: &Connection,
    context_id: i64,
    aggregate: &ContextAggregate,
) -> Result<()> {
    // Exe targets: single-owner by design; assign moves them, extras are
    // removed — EXCEPT a row this device already resolved for its own
    // platform (a real installed app, not the "?::" unresolved marker) is
    // sticky: `resolve_context_targets_in_ops` reruns app matching on every
    // incoming op and always stamps the result with this device's own
    // platform tag, so without this guard a stale/failed re-match on a later
    // sync would silently delete a target the user (or an earlier successful
    // match) already pinned correctly on this device. A genuinely
    // cross-device removal only reaches this device through its own local
    // `remove_context_target` call, never through this reconcile path, so
    // protecting sticky rows here never blocks a real local delete.
    let my_platform = db::current_platform_tag();
    let sticky_executables: std::collections::HashSet<String> = conn
        .prepare(
            "SELECT executable FROM context_targets
               WHERE context_id = ?1
                 AND (platform = ?2 OR (?2 IS NULL AND platform IS NULL))
                 AND executable NOT LIKE '?::%'",
        )?
        .query_map(params![context_id, my_platform], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    for entry in &aggregate.targets {
        let normalized = entry.executable().trim().to_lowercase();
        if normalized.is_empty() || sticky_executables.contains(&normalized) {
            continue;
        }
        conn.execute(
            "INSERT INTO context_targets (context_id, executable, app_name, developer, platform)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(executable) DO UPDATE SET context_id = excluded.context_id,
                 app_name = excluded.app_name, developer = excluded.developer,
                 platform = excluded.platform",
            params![
                context_id,
                normalized,
                entry.app_name(),
                entry.developer(),
                entry.platform()
            ],
        )?;
    }
    let target_executables: Vec<String> = aggregate
        .targets
        .iter()
        .map(|entry| entry.executable().to_string())
        .chain(sticky_executables.iter().cloned())
        .collect();
    remove_missing(
        conn,
        "context_targets",
        "executable",
        context_id,
        &target_executables,
    )?;
    let normalized_websites = aggregate
        .websites
        .iter()
        .map(|domain| normalize_domain(domain))
        .collect::<Vec<_>>();
    for normalized in &normalized_websites {
        if normalized.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT INTO context_website_targets (context_id, domain) VALUES (?1, ?2)
             ON CONFLICT(domain) DO UPDATE SET context_id = excluded.context_id",
            params![context_id, normalized],
        )?;
    }
    remove_missing(
        conn,
        "context_website_targets",
        "domain",
        context_id,
        &normalized_websites,
    )?;
    reconcile_context_members(conn, context_id, aggregate)?;
    Ok(())
}

/// Junction membership reconcile: add everything in the payload that resolves
/// locally, remove everything currently present that the payload lacks. A
/// dictionary reference carries its natural term as well as its UUID so a
/// canonical loser can still resolve to the surviving shared row.
fn reconcile_context_members(
    conn: &Connection,
    context_id: i64,
    aggregate: &ContextAggregate,
) -> Result<()> {
    let dictionary_entries = if aggregate.dictionary_entries.is_empty() {
        aggregate
            .dictionary_uuids
            .iter()
            .map(|uuid| DictionaryReference {
                uuid: uuid.clone(),
                term: String::new(),
            })
            .collect::<Vec<_>>()
    } else {
        aggregate.dictionary_entries.clone()
    };
    let mut resolved_dictionary: Vec<i64> = Vec::with_capacity(dictionary_entries.len());
    let mut unresolved_dictionary = false;
    {
        let mut stmt = conn.prepare("SELECT id FROM dictionary WHERE uuid = ?1")?;
        for entry in &dictionary_entries {
            let by_uuid = stmt
                .query_row(params![entry.uuid], |r| r.get::<_, i64>(0))
                .optional()?;
            let by_term = if by_uuid.is_none() && !entry.term.trim().is_empty() {
                conn.query_row(
                    "SELECT id FROM dictionary WHERE term = ?1",
                    params![entry.term.trim()],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?
            } else {
                None
            };
            if let Some(id) = by_uuid.or(by_term) {
                if !resolved_dictionary.contains(&id) {
                    resolved_dictionary.push(id);
                }
            } else {
                unresolved_dictionary = true;
            }
        }
    }
    for dictionary_id in &resolved_dictionary {
        conn.execute(
            "INSERT OR IGNORE INTO dictionary_contexts (context_id, dictionary_id) VALUES (?1, ?2)",
            params![context_id, dictionary_id],
        )?;
    }
    // Do not prune on an unresolved reference: the parent may be in a later
    // batch, and pruning now would erase a valid local assignment. Once every
    // reference resolves, record correction tombstones before the junction
    // delete (the dictionary_contexts delete trigger removes child mappings).
    if !unresolved_dictionary {
        let correction_uuids =
            correction_uuids_for_pruned_dictionary_members(conn, context_id, &resolved_dictionary)?;
        delete_members_not_in(
            conn,
            "dictionary_contexts",
            "dictionary_id",
            context_id,
            &resolved_dictionary,
        )?;
        for correction_uuid in correction_uuids {
            append_self_log(conn, "dictionary_corrections", &correction_uuid, "delete")?;
        }
    }

    let mut resolved_snippets: Vec<i64> = Vec::with_capacity(aggregate.snippet_uuids.len());
    {
        let mut stmt = conn.prepare("SELECT id FROM snippets WHERE uuid = ?1")?;
        for uuid in &aggregate.snippet_uuids {
            if let Some(id) = stmt
                .query_row(params![uuid], |r| r.get::<_, i64>(0))
                .optional()?
            {
                resolved_snippets.push(id);
            }
        }
    }
    for snippet_id in &resolved_snippets {
        conn.execute(
            "INSERT OR IGNORE INTO snippet_contexts (context_id, snippet_id) VALUES (?1, ?2)",
            params![context_id, snippet_id],
        )?;
    }
    delete_members_not_in(
        conn,
        "snippet_contexts",
        "snippet_id",
        context_id,
        &resolved_snippets,
    )?;
    Ok(())
}

fn correction_uuids_for_pruned_dictionary_members(
    conn: &Connection,
    context_id: i64,
    keep_ids: &[i64],
) -> Result<Vec<String>> {
    let keep_json = serde_json::to_string(keep_ids)?;
    conn.prepare(
        "SELECT c.uuid
           FROM dictionary_corrections c
          WHERE c.context_id = ?1
            AND c.uuid IS NOT NULL
            AND c.dictionary_id NOT IN (SELECT value FROM json_each(?2))",
    )?
    .query_map(params![context_id, keep_json], |r| r.get::<_, String>(0))?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(Into::into)
}

fn delete_members_not_in(
    conn: &Connection,
    table: &str,
    column: &str,
    context_id: i64,
    keep_ids: &[i64],
) -> Result<()> {
    // json_each keeps a dynamic NOT IN list to a single bound parameter.
    let keep_json = serde_json::to_string(keep_ids)?;
    let n = conn.execute(
        &format!(
            "DELETE FROM {table}
             WHERE context_id = ?1
               AND {column} NOT IN (SELECT value FROM json_each(?2))"
        ),
        params![context_id, keep_json],
    )?;
    if n > 0 {
        log::debug!("sync: pruned {n} stale rows from {table} for context {context_id}");
    }
    Ok(())
}

fn remove_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    context_id: i64,
    keep_values: &[String],
) -> Result<()> {
    let keep_json = serde_json::to_string(
        &keep_values
            .iter()
            .map(|s| s.trim().to_lowercase())
            .collect::<Vec<_>>(),
    )?;
    let n = conn.execute(
        &format!(
            "DELETE FROM {table}
             WHERE context_id = ?1
               AND {column} NOT IN (SELECT value FROM json_each(?2))"
        ),
        params![context_id, keep_json],
    )?;
    if n > 0 {
        log::debug!("sync: pruned {n} stale rows from {table} for context {context_id}");
    }
    Ok(())
}

fn normalize_domain(domain: &str) -> String {
    let trimmed = domain.trim().to_lowercase();
    trimmed
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_start_matches("www.")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn apply_transcription_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() {
        // Transcription deletion is retention cleanup, and retention is
        // intentionally device-local. A peer must never erase a row merely
        // because another device has a shorter local retention window.
        return Ok(Applied::Skipped);
    }
    if let Some(stamp) = latest_stamp(conn, "transcriptions", &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    let row: TranscriptionRow = serde_json::from_value(
        op.payload
            .clone()
            .ok_or_else(|| anyhow!("transcription upsert missing payload"))?,
    )
    .context("invalid transcription payload")?;
    let context_id: Option<i64> = match &row.context_uuid {
        Some(context_uuid) => conn
            .query_row(
                "SELECT id FROM contexts WHERE uuid = ?1",
                params![context_uuid],
                |r| r.get(0),
            )
            .optional()?,
        None => None,
    };
    // Raw insert on purpose: lifetime counters must NOT be bumped here - the
    // dictation was already counted by the device it happened on, and its
    // counters arrive through the stats exchange.
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO transcriptions
           (uuid, raw_text, clean_text, words, spoken_words, duration_ms, api_used, app_name, context_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            op.row_uuid,
            row.raw_text,
            row.clean_text,
            row.words,
            row.spoken_words,
            row.duration_ms,
            row.api_used,
            row.app_name,
            context_id,
            row.created_at
        ],
    )?;
    if inserted > 0 {
        log_applied(conn, op)?;
        Ok(Applied::Yes)
    } else {
        // Row already existed with an older stamp (e.g. pre-uuid history).
        // Don't log: our newer local version stays authoritative.
        Ok(Applied::Skipped)
    }
}

fn apply_api_call_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() {
        return apply_simple_delete(conn, op, "api_calls", "api_calls");
    }
    if let Some(stamp) = latest_stamp(conn, "api_calls", &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    let row: ApiCallRow = serde_json::from_value(
        op.payload
            .clone()
            .ok_or_else(|| anyhow!("api_call upsert missing payload"))?,
    )
    .context("invalid api call payload")?;
    let Some(transcription_uuid) = row.transcription_uuid.as_deref() else {
        log::warn!(
            "sync: skipping api call {} without a transcription",
            op.row_uuid
        );
        return Ok(Applied::Skipped);
    };
    let Some(transcription_id) = conn
        .query_row(
            "SELECT id FROM transcriptions WHERE uuid = ?1",
            params![transcription_uuid],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        log::warn!(
            "sync: skipping api call {} with missing transcription {}",
            op.row_uuid,
            transcription_uuid
        );
        return Ok(Applied::Skipped);
    };
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO api_calls
           (uuid, transcription_id, model, provider, task, audio_ms, input_chars, output_chars, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            op.row_uuid,
            transcription_id,
            row.model,
            row.provider,
            row.task,
            row.audio_ms,
            row.input_chars,
            row.output_chars,
            row.created_at
        ],
    )?;
    if inserted > 0 {
        log_applied(conn, op)?;
        Ok(Applied::Yes)
    } else {
        Ok(Applied::Skipped)
    }
}

// ---------------------------------------------------------------------------
// Meta exchange (stats + settings)
// ---------------------------------------------------------------------------

pub fn build_stats_exchange(conn: &Connection, self_uuid: &str) -> Result<StatsExchange> {
    let (total_words, dictionary_fixes) = conn
        .query_row(
            "SELECT COALESCE(total_words, 0), COALESCE(dictionary_fixes, 0)
             FROM lifetime_stats WHERE id = 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?
        .unwrap_or((0, 0));
    let remote_stats = sync_store::list_remote_stats(conn)?
        .into_iter()
        .map(|s| DeviceStatsDto {
            device_id: s.device_id,
            total_words: s.total_words,
            dictionary_fixes: s.dictionary_fixes,
        })
        .collect();
    Ok(StatsExchange {
        self_stats: DeviceStatsDto {
            device_id: self_uuid.to_string(),
            total_words,
            dictionary_fixes,
        },
        remote_stats,
    })
}

/// Applies the peer's stats exchange: their own counters and everything they
/// know about third devices. Rows for ourselves are ignored (our own
/// `lifetime_stats` row is authoritative for this device).
pub fn apply_stats_exchange(
    conn: &Connection,
    stats: &StatsExchange,
    peer_uuid: &str,
) -> Result<()> {
    let self_uuid = sync_store::self_uuid(conn)?.unwrap_or_default();
    let mut incoming: Vec<DeviceStatsDto> = Vec::with_capacity(stats.remote_stats.len() + 1);
    incoming.push(stats.self_stats.clone());
    incoming.extend(stats.remote_stats.iter().cloned());
    for dto in incoming {
        if dto.device_id == self_uuid || dto.device_id.is_empty() {
            continue;
        }
        let _ = peer_uuid; // peer's own row arrives as dto.self_stats
        sync_store::upsert_remote_stats(
            conn,
            &super::store::DeviceStats {
                device_id: dto.device_id,
                total_words: dto.total_words,
                dictionary_fixes: dto.dictionary_fixes,
            },
        )?;
    }
    Ok(())
}

/// Applies the peer's settings with per-key LWW. An unstamped local key uses
/// the zero stamp until it is changed or a remote value is accepted, so an
/// incoming stamped value can win the first exchange.
pub fn apply_settings_exchange(
    conn: &Connection,
    host: &dyn SyncHost,
    settings: &[SettingRecord],
) -> Result<usize> {
    let mut accepted = Vec::new();
    for record in settings {
        if !SYNCABLE_SETTINGS.contains(&record.key.as_str()) {
            continue;
        }
        let local = sync_store::get_setting_stamp(conn, &record.key)?;
        let local_stamp = local
            .map(|s| (s.ts_ms, s.origin))
            .unwrap_or((0, String::new()));
        if (record.ts_ms, record.origin.as_str()) <= (local_stamp.0, local_stamp.1.as_str()) {
            continue;
        }
        accepted.push(record);
    }
    if accepted.is_empty() {
        return Ok(0);
    }
    let values: Vec<(String, serde_json::Value)> = accepted
        .iter()
        .map(|record| (record.key.clone(), record.value.clone()))
        .collect();
    if let Err(err) = host.apply_remote_settings(&values) {
        log::warn!("sync: failed to apply settings batch: {err}");
        return Ok(0);
    }
    for record in accepted {
        sync_store::set_setting_stamp(conn, &record.key, record.ts_ms, &record.origin)?;
    }
    Ok(values.len())
}

/// Stamps a local settings change so it wins LWW against peers from now on.
/// Called from the save_setting command path for syncable keys.
pub fn record_local_setting_change(conn: &Connection, key: &str) -> Result<()> {
    if !SYNCABLE_SETTINGS.contains(&key) {
        return Ok(());
    }
    sync_store::set_setting_stamp(
        conn,
        key,
        sync_store::now_ms(),
        &sync_store::self_uuid(conn)?.unwrap_or_default(),
    )
}

// ---------------------------------------------------------------------------
// Session driver
// ---------------------------------------------------------------------------

/// What a completed session did, for status events.
#[derive(Debug, Default)]
pub struct SessionSummary {
    pub applied: ApplySummary,
    pub settings_applied: usize,
}

fn hello(uuid: &str, name: &str, app_version: &str) -> Hello {
    Hello {
        device_uuid: uuid.to_string(),
        device_name: name.to_string(),
        protocol: PROTOCOL_VERSION,
        app_version: app_version.to_string(),
    }
}

/// Runs one full sync session over an authenticated stream. Both sides call
/// this; the phases run in a fixed order (initiator pulls, initiator serves,
/// then the roles flip), so the sequential message flow cannot deadlock:
///
/// 1. Hello exchange (initiator sends first).
/// 2. Meta exchange - stats + settings, initiator sends first.
/// 3. Initiator pulls from the responder (request/batches/acks).
/// 4. Responder pulls from the initiator.
/// 5. Each side sends SyncDone and drains the peer's.
///
/// The caller must already have verified the peer: TLS is up, the `Hello`
/// uuid maps to a paired peer, and the certificate fingerprint matches the
/// pin recorded at pairing time.
pub async fn run_session<S>(
    db: &DbHandle,
    host: &dyn SyncHost,
    stream: &mut S,
    initiator: bool,
    peer: &SyncPeer,
) -> Result<SessionSummary>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    run_session_after_hello(db, host, stream, initiator, peer, None).await
}

/// Responder entry point when the connection dispatcher already consumed and
/// authenticated the initial Hello frame.
pub async fn run_session_after_hello<S>(
    db: &DbHandle,
    host: &dyn SyncHost,
    stream: &mut S,
    initiator: bool,
    peer: &SyncPeer,
    remote_hello: Option<Hello>,
) -> Result<SessionSummary>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use super::protocol::{read_message, send_message};

    // 1. Hello exchange.
    if initiator {
        send_message(
            stream,
            &Message::Hello(hello(
                &host.device_uuid(),
                &host.device_name(),
                &host.app_version(),
            )),
        )
        .await?;
        match read_message(stream).await? {
            Message::HelloAck(remote) => check_hello(&remote)?,
            Message::Error { message } => return Err(anyhow!("peer error: {message}")),
            other => return Err(anyhow!("expected HelloAck, got {other:?}")),
        }
    } else {
        if let Some(remote) = remote_hello {
            check_hello(&remote)?;
        } else {
            match read_message(stream).await? {
                Message::Hello(remote) => check_hello(&remote)?,
                Message::Error { message } => return Err(anyhow!("peer error: {message}")),
                other => return Err(anyhow!("expected Hello, got {other:?}")),
            }
        }
        send_message(
            stream,
            &Message::HelloAck(hello(
                &host.device_uuid(),
                &host.device_name(),
                &host.app_version(),
            )),
        )
        .await?;
    }

    let mut summary = SessionSummary::default();

    // 2. Meta exchange (stats + settings), initiator first. The DB lock is
    // never held across an await (the sync DB is shared with the pipeline).
    let stats = {
        let conn = lock(db)?;
        build_stats_exchange(&conn, &host.device_uuid())?
    };
    // ManagerHost reads setting stamps from this same database. Calling it
    // while holding `conn` deadlocks because std::sync::Mutex is not reentrant.
    let meta = Message::Meta {
        stats,
        settings: host.settings_payload()?,
    };
    if initiator {
        send_message(stream, &meta).await?;
    }
    let remote_meta = match read_message(stream).await? {
        Message::Meta { stats, settings } => (stats, settings),
        Message::Error { message } => return Err(anyhow!("peer error: {message}")),
        other => return Err(anyhow!("expected Meta, got {other:?}")),
    };
    if !initiator {
        send_message(stream, &meta).await?;
    }
    {
        let conn = lock(db)?;
        apply_stats_exchange(&conn, &remote_meta.0, peer.device_uuid.as_str())?;
        summary.settings_applied = apply_settings_exchange(&conn, host, &remote_meta.1)?;
    }
    if summary.settings_applied > 0 {
        summary.applied.settings = true;
    }
    summary.applied.stats = true;

    // 3 + 4. Pull both directions in fixed order.
    let mut pulled = ApplySummary::default();
    if initiator {
        pulled.merge(pull_from_peer(db, host, stream, peer).await?);
        serve_peer_pulls(db, stream, peer).await?;
    } else {
        serve_peer_pulls(db, stream, peer).await?;
        pulled.merge(pull_from_peer(db, host, stream, peer).await?);
    }
    summary.applied.merge(pulled);

    // 5. Done: announce and drain the peer's announcement (or their close).
    let _ = send_message(stream, &Message::SyncDone).await;
    match read_message(stream).await {
        Ok(Message::SyncDone) | Err(_) => {}
        Ok(_) => {}
    }
    Ok(summary)
}

fn check_hello(remote: &Hello) -> Result<()> {
    if remote.protocol != PROTOCOL_VERSION {
        return Err(anyhow!(
            "peer runs sync protocol v{} but this device speaks v{}",
            remote.protocol,
            PROTOCOL_VERSION
        ));
    }
    Ok(())
}

fn lock(db: &DbHandle) -> Result<std::sync::MutexGuard<'_, Connection>> {
    db.lock().map_err(|_| anyhow!("database lock was poisoned"))
}

/// Puller side: request deltas, apply batches until the sender reports a
/// final cursor. A SyncDone from the peer mid-pull is a clean terminator.
async fn pull_from_peer<S>(
    db: &DbHandle,
    host: &dyn SyncHost,
    stream: &mut S,
    peer: &SyncPeer,
) -> Result<ApplySummary>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut summary = ApplySummary::default();
    // The pull position is OUR record of the peer's log (recv_cursor) - the
    // send_cursor on the same row tracks the opposite direction.
    let since = {
        let conn = lock(db)?;
        sync_store::peer_recv_cursor(&conn, &peer.device_uuid)?
    };
    let snapshot = since == 0;
    send_message(
        stream,
        &Message::PullRequest(PullRequest {
            since_seq: since,
            snapshot,
        }),
    )
    .await?;

    loop {
        match read_message(stream).await? {
            Message::Ops(batch) => {
                let resolved_ops = resolve_context_targets_in_ops(host, &batch.ops);
                let applied = {
                    let conn = lock(db)?;
                    apply_ops(&conn, &resolved_ops)?
                };
                summary.merge(applied);
                let acked = batch.cursor;
                send_message(stream, &Message::Ack { seq: acked }).await?;
                if batch.done {
                    // Final batch: remember the position so the next session
                    // pulls only fresh changes. A deferred correction keeps
                    // the old cursor, causing the sender to replay the
                    // dependency and child rather than permanently dropping
                    // the child when it was in an earlier batch.
                    if !summary.deferred {
                        let conn = lock(db)?;
                        sync_store::set_peer_recv_cursor(&conn, &peer.device_uuid, acked)?;
                    } else {
                        log::warn!(
                            "sync: retaining peer cursor because a correction dependency was deferred"
                        );
                    }
                    break;
                }
            }
            Message::SyncDone => break,
            Message::Error { message } => return Err(anyhow!("peer error: {message}")),
            other => return Err(anyhow!("unexpected message during pull: {other:?}")),
        }
    }
    Ok(summary)
}

/// Sender side: serve the peer's pull request with batches until it acks the
/// final cursor. Returns when the peer's pull is complete - it will then start
/// its own pull.
async fn serve_peer_pulls<S>(db: &DbHandle, stream: &mut S, peer: &SyncPeer) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match super::protocol::read_message(stream).await? {
        Message::PullRequest(request) => {
            serve_one_pull(db, stream, peer, request.since_seq, request.snapshot).await
        }
        Message::SyncDone => Ok(()),
        Message::Error { message } => Err(anyhow!("peer error: {message}")),
        other => Err(anyhow!("expected PullRequest, got {other:?}")),
    }
}

async fn serve_one_pull<S>(
    db: &DbHandle,
    stream: &mut S,
    peer: &SyncPeer,
    since_seq: i64,
    requested_snapshot: bool,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let snapshot = if requested_snapshot {
        true
    } else {
        let conn = lock(db)?;
        sync_store::needs_snapshot_for(&conn, &peer.device_uuid, since_seq)?
    };
    let mut progress = SnapshotProgress::default();
    let mut next_since_seq = since_seq;
    loop {
        let (ops, cursor, done) = {
            let conn = lock(db)?;
            collect_ops(
                &conn,
                next_since_seq,
                snapshot,
                OPS_PER_BATCH,
                &mut progress,
            )?
        };
        let final_cursor = cursor;
        send_message(
            stream,
            &Message::Ops(OpsBatch {
                ops,
                cursor: final_cursor,
                done,
                snapshot,
            }),
        )
        .await?;
        match read_message(stream).await? {
            Message::Ack { seq } => {
                let conn = lock(db)?;
                sync_store::set_peer_send_position(
                    &conn,
                    &peer.device_uuid,
                    seq,
                    snapshot && !done,
                )?;
            }
            Message::Error { message } => return Err(anyhow!("peer error: {message}")),
            other => return Err(anyhow!("unexpected message during serve: {other:?}")),
        }
        if !snapshot && !done {
            next_since_seq = cursor;
        }
        if done {
            break;
        }
    }
    Ok(())
}

impl ApplySummary {
    fn merge(&mut self, other: ApplySummary) {
        self.dictionary |= other.dictionary;
        self.dictionary_corrections |= other.dictionary_corrections;
        self.snippets |= other.snippets;
        self.contexts |= other.contexts;
        self.history |= other.history;
        self.settings |= other.settings;
        self.stats |= other.stats;
        self.deferred |= other.deferred;
        self.applied += other.applied;
        self.skipped += other.skipped;
    }
}
