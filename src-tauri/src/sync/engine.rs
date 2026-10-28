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
use super::store::{latest_op_stamp as latest_stamp, SyncPeer};

#[derive(Debug, Default, Clone, Copy)]
pub struct SnapshotProgress {
    pub stage: u8,
    pub last_id: i64,
    pub origin_seq: Option<i64>,
    pub start_cursor: Option<i64>,
}

pub const SYNCABLE_SETTINGS: &[&str] = &[
    store::TRANSCRIPTION_LANGUAGE,
    store::CLEANUP_ENABLED,
    store::VOICE_COMMANDS_ENABLED,
    store::DEFAULT_TONE,
    store::CLEANUP_INTENSITY,
    store::AUTO_LEARN_ENABLED,
    store::AUTO_LEARN_EVENT_MODE,
    store::CONTEXTUAL_FORMATTING,
    store::CLEANUP_PROMPT_OVERRIDE,
    store::VERENU_SERVICE_CHECKS_ENABLED,
];

const UNRESOLVED_APP_PREFIX: &str = "?::";
const NATURAL_KEY_TOMBSTONE_TABLE: &str = "dictionary_natural_key";

type ExistingCorrection = (i64, Option<String>, bool, i64, String, Option<String>);
type CorrectionTarget = (i64, Option<String>, i64, bool);
type CorrectionIdentity = (i64, i64, i64, String, bool);

pub trait SyncHost: Send + Sync {
    fn device_uuid(&self) -> String;
    fn device_name(&self) -> String;
    fn app_version(&self) -> String;
    fn history_retention_days(&self) -> Option<i64> { None }
    fn settings_payload(&self) -> Result<Vec<SettingRecord>>;
    fn apply_remote_setting(&self, key: &str, value: &serde_json::Value) -> Result<(), String>;
    fn apply_remote_settings(
        &self,
        settings: &[(String, serde_json::Value)],
    ) -> Result<(), String> {
        for (key, value) in settings {
            self.apply_remote_setting(key, value)?;
        }
        Ok(())
    }
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

fn resolve_context_targets_in_ops(host: &dyn SyncHost, ops: Vec<SyncOp>) -> Vec<SyncOp> {
    ops.into_iter()
        .map(|mut op| {
            if op.table == "context_sub_apps" && op.op != "delete" {
                if let Some(map) = op.payload.as_mut().and_then(serde_json::Value::as_object_mut) {
                    if let Some(source) = map.get("executable").and_then(|v| v.as_str()).map(str::to_string) {
                        let app_name = map.get("app_name").and_then(|v| v.as_str()).map(str::to_string);
                        let (resolved, resolved_name) = host
                            .resolve_app_target_with_metadata(&source, app_name.as_deref(), None)
                            .map(|(resolved, name, _)| (resolved, name))
                            .unwrap_or_else(|| (unresolved_app_target(&source), app_name.clone()));
                        let platform = if resolved.starts_with(UNRESOLVED_APP_PREFIX) {
                            None
                        } else {
                            db::current_platform_tag()
                        };
                        map.insert("executable".into(), serde_json::json!(resolved));
                        map.insert("app_name".into(), serde_json::json!(resolved_name));
                        map.insert("platform".into(), serde_json::json!(platform));
                    }
                }
                return op;
            }
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
    pub paste_in_chunks: bool,
    #[serde(default)]
    pub t3_skill_mentions_disabled: Option<bool>,
    pub contextual_formatting_override: Option<bool>,
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
    #[serde(default)]
    pub dictionary_entries: Vec<DictionaryReference>,
    #[serde(default)]
    pub snippet_uuids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubAppRow {
    pub executable: String,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub title_pattern: String,
    #[serde(default = "default_match_mode")]
    pub match_mode: String,
    #[serde(default)]
    pub context_uuid: Option<String>,
    pub created_at: String,
}

fn default_match_mode() -> String {
    "contains".to_string()
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
    #[serde(default)]
    pub activity_day: Option<String>,
    #[serde(default)]
    pub activity_hour: Option<i64>,
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
    #[serde(default)]
    pub activity_day: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ApplySummary {
    pub dictionary: bool,
    pub dictionary_corrections: bool,
    pub snippets: bool,
    pub contexts: bool,
    pub history: bool,
    pub settings: bool,
    pub stats: bool,
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

fn latest_op_name(conn: &Connection, table: &str, row_uuid: &str) -> Result<Option<String>> {
    conn.prepare_cached(
        "SELECT op FROM sync_log
          WHERE table_name = ?1 AND row_uuid = ?2
          ORDER BY seq DESC LIMIT 1",
    )?
    .query_row(
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

macro_rules! row_payloads {
    ($($name:ident, $ty:ident, $message:literal, $sql:literal,
        |$r:ident| { $($field:ident: $value:expr),* $(,)? };)*) => {
        $(fn $name(conn: &Connection, uuid: &str) -> Result<Option<serde_json::Value>> {
            let row = conn.prepare_cached($sql)?
                .query_row(params![uuid], |$r| Ok($ty { $($field: $value),* }))
                .optional()?;
            Ok(row.map(|row| serde_json::to_value(row).expect($message)))
        })*
    };
}

row_payloads! {
    dictionary_payload, DictionaryRow, "serialize dictionary row",
    "SELECT term, mistake, auto_learned, correction_count, confidence_tier, last_seen_at, created_at
     FROM dictionary WHERE uuid = ?1",
    |r| {
        term: r.get(0)?,
        mistake: None,
        auto_learned: r.get::<_, i64>(2)? != 0,
        correction_count: r.get(3)?,
        confidence_tier: r.get(4)?,
        last_seen_at: r.get(5)?,
        created_at: r.get(6)?,
    };
    dictionary_correction_payload, DictionaryCorrectionRow, "serialize dictionary correction row",
    "SELECT COALESCE(ctx.uuid, ''), COALESCE(d.uuid, ''), d.term, c.mistake, c.auto_learned,
            c.correction_count, c.confidence_tier, c.last_seen_at, c.created_at
       FROM dictionary_corrections c
       INNER JOIN contexts ctx ON ctx.id = c.context_id
       INNER JOIN dictionary d ON d.id = c.dictionary_id
      WHERE c.uuid = ?1",
    |r| {
        context_uuid: r.get(0)?,
        dictionary_uuid: r.get(1)?,
        dictionary_term: r.get(2)?,
        mistake: r.get(3)?,
        auto_learned: r.get::<_, i64>(4)? != 0,
        correction_count: r.get(5)?,
        confidence_tier: r.get(6)?,
        last_seen_at: r.get(7)?,
        created_at: r.get(8)?,
    };
    snippet_payload, SnippetRow, "serialize snippet row",
    "SELECT trigger, expansion, instructions, use_count, created_at
     FROM snippets WHERE uuid = ?1",
    |r| {
        trigger: r.get(0)?,
        expansion: r.get(1)?,
        instructions: r.get(2)?,
        use_count: r.get(3)?,
        created_at: r.get(4)?,
    };
    sub_app_payload, SubAppRow, "serialize sub-app row",
    "SELECT s.executable, s.platform, s.app_name, s.label, s.icon, s.title_pattern,
            s.match_mode, c.uuid, s.created_at
       FROM context_sub_apps s LEFT JOIN contexts c ON c.id = s.context_id
      WHERE s.uuid = ?1",
    |r| {
        executable: r.get(0)?,
        platform: r.get(1)?,
        app_name: r.get(2)?,
        label: r.get(3)?,
        icon: r.get(4)?,
        title_pattern: r.get(5)?,
        match_mode: r.get(6)?,
        context_uuid: r.get(7)?,
        created_at: r.get(8)?,
    };
    transcription_payload, TranscriptionRow, "serialize transcription row",
    "SELECT t.raw_text, t.clean_text, t.words, t.spoken_words, t.duration_ms, t.api_used,
            t.app_name, COALESCE(c.uuid, p.context_uuid), t.created_at, t.activity_day, t.activity_hour
     FROM transcriptions t LEFT JOIN contexts c ON c.id = t.context_id
     LEFT JOIN pending_transcription_contexts p ON p.transcription_uuid = t.uuid
     WHERE t.uuid = ?1",
    |r| {
        raw_text: r.get(0)?,
        clean_text: r.get(1)?,
        words: r.get(2)?,
        spoken_words: r.get(3)?,
        duration_ms: r.get(4)?,
        api_used: r.get(5)?,
        app_name: r.get(6)?,
        context_uuid: r.get(7)?,
        created_at: r.get(8)?,
        activity_day: r.get(9)?,
        activity_hour: r.get(10)?,
    };
    api_call_payload, ApiCallRow, "serialize api call row",
    "SELECT t.uuid, a.model, a.provider, a.task, a.audio_ms, a.input_chars, a.output_chars, a.created_at, a.activity_day
     FROM api_calls a LEFT JOIN transcriptions t ON t.id = a.transcription_id
     WHERE a.uuid = ?1",
    |r| {
        transcription_uuid: r.get(0)?,
        model: r.get(1)?,
        provider: r.get(2)?,
        task: r.get(3)?,
        audio_ms: r.get(4)?,
        input_chars: r.get(5)?,
        output_chars: r.get(6)?,
        created_at: r.get(7)?,
        activity_day: r.get(8)?,
    };
}

fn context_aggregate(conn: &Connection, uuid: &str) -> Result<Option<serde_json::Value>> {
    let row = conn
        .prepare_cached(
            "SELECT id, name, is_everywhere, icon, tone, cleanup_intensity, color, custom_instructions,
                    contextual_formatting_disabled, pinned_at, created_at, updated_at, paste_in_chunks, t3_skill_mentions_disabled, contextual_formatting_override
             FROM contexts WHERE uuid = ?1",
        )?
        .query_row(
            params![uuid],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    ContextAggregate {
                        name: r.get(1)?,
                        is_everywhere: r.get::<_, i64>(2)? != 0,
                        icon: r.get(3)?,
                        tone: r.get(4)?,
                        cleanup_intensity: r.get(5)?,
                        color: r.get(6)?,
                        custom_instructions: r.get(7)?,
                        contextual_formatting_disabled: r.get::<_, i64>(8)? != 0,
                        pinned_at: r.get(9)?,
                        created_at: r.get(10)?,
                        updated_at: r.get(11)?,
                        paste_in_chunks: r.get::<_, i64>(12)? != 0,
                        t3_skill_mentions_disabled: Some(r.get::<_, bool>(13)?),
                        contextual_formatting_override: r.get(14)?,
                        targets: Vec::new(),
                        websites: Vec::new(),
                        dictionary_uuids: Vec::new(),
                        dictionary_entries: Vec::new(),
                        snippet_uuids: Vec::new(),
                    },
                ))
            },
        )
        .optional()?;
    let (context_id, mut aggregate) = match row {
        Some(row) => row,
        None => return Ok(None),
    };
    let mut stmt = conn.prepare_cached(
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
    let mut stmt = conn.prepare_cached(
        "SELECT domain FROM context_website_targets WHERE context_id = ?1 ORDER BY domain",
    )?;
    aggregate.websites = stmt
        .query_map(params![context_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare_cached(
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
    let mut stmt = conn.prepare_cached(
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

fn row_payload(conn: &Connection, table: &str, uuid: &str) -> Result<Option<serde_json::Value>> {
    let mut payload = match table {
        "dictionary" => dictionary_payload(conn, uuid),
        "dictionary_corrections" => dictionary_correction_payload(conn, uuid),
        "snippets" => snippet_payload(conn, uuid),
        "contexts" => context_aggregate(conn, uuid),
        "context_sub_apps" => sub_app_payload(conn, uuid),
        "transcriptions" | "held_transcriptions" | "held_hourly_transcriptions" => transcription_payload(conn, uuid),
        "api_calls" | "held_api_calls" => api_call_payload(conn, uuid),
        "transcription_receipts" => conn.query_row("SELECT day, hour, context_uuid, words, spoken_words, duration_ms, counted, NOT EXISTS(SELECT 1 FROM backup_held_hourly_receipts h WHERE h.uuid=transcription_receipts.uuid) FROM transcription_receipts WHERE uuid=?1 AND counted=1", [uuid], |r| Ok(serde_json::json!({"uuid":uuid, "day":r.get::<_, String>(0)?, "hour":r.get::<_, i64>(1)?, "context_uuid":r.get::<_, Option<String>>(2)?, "words":r.get::<_, i64>(3)?, "spoken_words":r.get::<_, Option<i64>>(4)?, "duration_ms":r.get::<_, i64>(5)?, "counted":r.get::<_, bool>(6)?, "hourly_counted":r.get::<_, bool>(7)?}))).optional().map_err(Into::into),
        "call_receipts" => conn.query_row("SELECT transcription_uuid, day, context_uuid, model, provider, task, audio_ms, input_chars, output_chars, counted FROM call_receipts WHERE uuid=?1 AND counted=1", [uuid], |r| Ok(serde_json::json!({"uuid":uuid, "transcription_uuid":r.get::<_, Option<String>>(0)?, "day":r.get::<_, String>(1)?, "context_uuid":r.get::<_, Option<String>>(2)?, "model":r.get::<_, String>(3)?, "provider":r.get::<_, String>(4)?, "task":r.get::<_, String>(5)?, "audio_ms":r.get::<_, i64>(6)?, "input_chars":r.get::<_, i64>(7)?, "output_chars":r.get::<_, i64>(8)?, "counted":r.get::<_, bool>(9)?}))).optional().map_err(Into::into),
        _ => Ok(None),
    }?;
    if matches!(table, "transcriptions" | "held_transcriptions" | "held_hourly_transcriptions" | "transcription_receipts" | "call_receipts") {
        if let Some(value) = payload.as_mut() {
            let parent = if table == "call_receipts" { value.get("transcription_uuid").and_then(|v| v.as_str()) } else { Some(uuid) };
            if let Some(parent) = parent {
                let mut stmt = conn.prepare("SELECT source_uuid FROM backup_context_event_sources WHERE transcription_uuid=?1 AND local_uuid IS ?2 ORDER BY source_uuid")?;
                let sources = stmt.query_map(params![parent,value.get("context_uuid").and_then(|v| v.as_str())], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                if !sources.is_empty() { value["context_sources"] = serde_json::json!(sources); }
            }
        }
    }
    Ok(payload)
}

fn sync_history_table<'a>(conn: &Connection, table: &'a str, uuid: &str) -> Result<&'a str> {
    let table = match table {
        "held_transcriptions" | "held_hourly_transcriptions" => "transcriptions",
        "held_api_calls" => "api_calls",
        other => other,
    };
    let query = match table {
        "transcriptions" => "SELECT EXISTS(SELECT 1 FROM backup_held_history WHERE uuid=?1 UNION ALL SELECT 1 FROM backup_preupgrade_history WHERE uuid=?1)",
        "api_calls" => "SELECT EXISTS(SELECT 1 FROM backup_held_calls WHERE uuid=?1 UNION ALL SELECT 1 FROM api_calls a JOIN transcriptions t ON t.id=a.transcription_id JOIN backup_preupgrade_history h ON h.uuid=t.uuid WHERE a.uuid=?1)",
        _ => return Ok(table),
    };
    if conn.query_row(query, [uuid], |r| r.get::<_, bool>(0))? {
        Ok(if table == "transcriptions" { "held_transcriptions" } else { "held_api_calls" })
    } else if table == "transcriptions" && conn.query_row("SELECT EXISTS(SELECT 1 FROM backup_held_hourly_receipts WHERE uuid=?1)", [uuid], |r| r.get::<_, bool>(0))? {
        Ok("held_hourly_transcriptions")
    } else {
        Ok(table)
    }
}

fn resolve_entry(conn: &Connection, entry: sync_store::LogEntry) -> Result<Option<SyncOp>> {
    if entry.table_name == "transcriptions" && entry.op == "delete" {
        return Ok(None);
    }
    let table = if entry.op == "upsert" { sync_history_table(conn, &entry.table_name, &entry.row_uuid)? } else { &entry.table_name };
    let payload = if entry.op == "upsert" {
        row_payload(conn, table, &entry.row_uuid)?
    } else {
        None
    };
    let op = if entry.op == "upsert" && payload.is_none() {
        if matches!(entry.table_name.as_str(), "transcriptions" | "held_transcriptions" | "held_hourly_transcriptions" | "api_calls" | "held_api_calls" | "transcription_receipts" | "call_receipts") {
            return Ok(None);
        }
        "delete".to_string()
    } else {
        entry.op
    };
    Ok(Some(SyncOp {
        table: table.to_string(),
        row_uuid: entry.row_uuid,
        op,
        ts_ms: entry.ts_ms,
        origin: entry.origin,
        origin_seq: entry.origin_seq,
        payload,
    }))
}

pub fn collect_ops(
    conn: &Connection,
    since_seq: i64,
    snapshot: bool,
    limit: usize,
    progress: &mut SnapshotProgress,
) -> Result<(Vec<SyncOp>, i64, bool)> {
    let limit = limit.max(1) as i64;
    if snapshot {
        let start_cursor = match progress.start_cursor {
            Some(cursor) => cursor,
            None => {
                let cursor = sync_store::max_log_seq(conn)?;
                progress.start_cursor = Some(cursor);
                cursor
            }
        };
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

        while (ops.len() as i64) < limit && progress.stage <= 9 {
            let capacity = limit - ops.len() as i64;
            let chunk = capacity.min(SNAPSHOT_ROW_CHUNK);
            let stage = progress.stage;
            let table = match stage {
                0 => "dictionary",
                1 => "snippets",
                2 => "contexts",
                3 => "context_sub_apps",
                4 => "dictionary_corrections",
                5 => "transcriptions",
                6 => "transcription_receipts",
                7 => "api_calls",
                8 => "call_receipts",
                9 => "tombstones",
                _ => unreachable!("snapshot stage is complete"),
            };
            if stage == 9 {
                let mut stmt = conn.prepare_cached(
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
                    progress.stage = 10;
                    progress.origin_seq = Some(origin_seq);
                    return Ok((ops, start_cursor, true));
                }
                progress.origin_seq = Some(origin_seq);
                return Ok((ops, 0, false));
            }
            let row_query = if matches!(table, "transcription_receipts" | "call_receipts") {
                format!("SELECT rowid, uuid FROM {table} WHERE rowid > ?1 AND counted=1 ORDER BY rowid LIMIT ?2")
            } else if table == "dictionary_corrections" {
                format!(
                    "SELECT id, uuid FROM {table}
                      WHERE id > ?1 AND uuid IS NOT NULL
                      ORDER BY id LIMIT ?2"
                )
            } else {
                format!("SELECT id, uuid FROM {table} WHERE id > ?1 ORDER BY id LIMIT ?2")
            };
            let mut stmt = conn.prepare_cached(&row_query)?;
            let rows = stmt
                .query_map(params![progress.last_id, chunk], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let fetched = rows.len() as i64;
            for (id, uuid) in rows {
                progress.last_id = id;
                let wire_table = sync_history_table(conn, table, &uuid)?;
                let payload = row_payload(conn, wire_table, &uuid)?;
                if payload.is_none() { continue; }
                let stamp = latest_stamp(conn, table, &uuid)?;
                push(wire_table, uuid, payload, stamp, &mut ops);
            }
            if fetched < chunk {
                progress.stage += 1;
                progress.last_id = 0;
                if progress.stage > 9 {
                    progress.origin_seq = Some(origin_seq);
                    return Ok((ops, start_cursor, true));
                }
            } else {
                progress.origin_seq = Some(origin_seq);
                return Ok((ops, 0, false));
            }
        }
        if progress.stage <= 9 {
            progress.origin_seq = Some(origin_seq);
            return Ok((ops, 0, false));
        }
        progress.origin_seq = Some(origin_seq);
        return Ok((ops, start_cursor, true));
    }

    let entries = sync_store::changes_since(conn, since_seq, limit)?;
    let mut ops = Vec::with_capacity(entries.len());
    let done = entries.len() < limit as usize;
    let cursor = if done {
        sync_store::max_log_seq(conn)?
    } else {
        entries.last().map(|entry| entry.seq).unwrap_or(since_seq)
    };
    for entry in entries {
        if let Some(op) = resolve_entry(conn, entry)? {
            ops.push(op);
        }
    }
    Ok((ops, cursor, done))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Applied {
    Yes,
    Skipped,
    Deferred,
}

fn apply_rank(op: &SyncOp) -> u8 {
    match (op.table.as_str(), op.is_delete()) {
        ("contexts", true)
        | ("dictionary", true)
        | ("snippets", true)
        | ("context_sub_apps", true) => 0,
        ("dictionary", false) => 10,
        (NATURAL_KEY_TOMBSTONE_TABLE, true) => 15,
        ("snippets", false) => 11,
        ("contexts", false) => 20,
        ("context_sub_apps", false) => 25,
        ("dictionary_corrections", _) => 30,
        ("transcriptions", false) => 40,
        ("held_transcriptions", false) => 40,
        ("held_hourly_transcriptions", false) => 40,
        ("transcription_receipts", false) => 40,
        ("api_calls", false) => 41,
        ("held_api_calls", false) => 41,
        ("call_receipts", false) => 41,
        ("transcriptions", true) | ("api_calls", true) => 50,
        _ => 60,
    }
}

#[cfg(test)]
pub fn apply_ops(conn: &Connection, ops: &[SyncOp]) -> Result<ApplySummary> {
    apply_ops_with_retention(conn, ops, None)
}

fn apply_ops_with_retention(conn: &Connection, ops: &[SyncOp], days: Option<i64>) -> Result<ApplySummary> {
    let cutoff = days.map(|days| (chrono::Utc::now().naive_utc() - chrono::Duration::days(days.max(1))).format("%Y-%m-%d %H:%M:%S").to_string());
    with_sync_savepoint(conn, |conn| {
        let summary = apply_ops_inner(conn, ops, cutoff.as_deref())?;
        if summary.history || summary.contexts { crate::commands::settings::validate_totals(conn)?; }
        Ok(summary)
    })
}

fn apply_ops_inner(conn: &Connection, ops: &[SyncOp], cutoff: Option<&str>) -> Result<ApplySummary> {
    let _guard = ApplyingGuard::new(conn)?;
    let mut summary = ApplySummary::default();
    let mut ordered: Vec<&SyncOp> = ops.iter().collect();
    ordered.sort_by_key(|op| apply_rank(op));
    for original in ordered {
        let mut normalized = None;
        if matches!(original.table.as_str(), "transcriptions" | "held_transcriptions" | "held_hourly_transcriptions" | "transcription_receipts" | "call_receipts") && !original.is_delete() {
            if let Some(uuid) = original.payload.as_ref().and_then(|payload| payload.get("context_uuid")).and_then(|value| value.as_str()) {
                let everywhere_alias: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sync_everywhere_aliases WHERE uuid=?1)", [uuid], |r| r.get(0))?;
                let imported_alias: Option<String> = conn.query_row("SELECT c.uuid FROM backup_context_aliases a JOIN contexts c ON c.uuid=a.local_uuid WHERE a.source_uuid=?1 AND NOT EXISTS(SELECT 1 FROM contexts WHERE uuid=?1)", [uuid], |r| r.get(0)).optional()?;
                let context_exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM contexts WHERE uuid=?1)", [uuid], |r| r.get(0))?;
                let local_everywhere: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM contexts WHERE uuid=?1 AND is_everywhere=1)", [uuid], |r| r.get(0))?;
                let parent = if original.table == "call_receipts" { original.payload.as_ref().and_then(|p| p.get("transcription_uuid")).and_then(|v| v.as_str()) } else { Some(original.row_uuid.as_str()) };
                let historical: Option<String> = if parent.is_some() {
                    conn.query_row("SELECT c.uuid FROM backup_context_event_sources e JOIN contexts c ON c.uuid=e.local_uuid WHERE e.transcription_uuid=?1 AND e.source_uuid=?2 AND (EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1 AND context_id=c.id) OR EXISTS(SELECT 1 FROM transcription_receipts WHERE uuid=?1 AND context_uuid=c.uuid) OR EXISTS(SELECT 1 FROM call_receipts WHERE transcription_uuid=?1 AND context_uuid=c.uuid))", params![parent,uuid], |r| r.get(0)).optional()?
                } else { None };
                let mut mapped_event = None;
                if (everywhere_alias || local_everywhere) && parent.is_some() {
                    let mut stmt = conn.prepare("SELECT e.source_uuid,c.uuid FROM backup_context_event_sources e JOIN contexts c ON c.uuid=e.local_uuid WHERE e.transcription_uuid=?1 AND NOT EXISTS(SELECT 1 FROM contexts WHERE uuid=e.source_uuid) AND (EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1 AND context_id=c.id) OR EXISTS(SELECT 1 FROM transcription_receipts WHERE uuid=?1 AND context_uuid=c.uuid) OR EXISTS(SELECT 1 FROM call_receipts WHERE transcription_uuid=?1 AND context_uuid=c.uuid)) ORDER BY e.source_uuid")?;
                    let rows = stmt.query_map([parent], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?;
                    for row in rows {
                        let (source,target) = row?;
                        if latest_is_delete(conn, "contexts", &source)? { mapped_event = Some(target); break; }
                    }
                }
                if historical.is_some() || imported_alias.is_some() || mapped_event.is_some() || everywhere_alias || (!context_exists && latest_is_delete(conn, "contexts", uuid)?) {
                    let target = if let Some(alias) = historical.or(imported_alias).or(mapped_event) { alias } else {
                        let id = db::ensure_everywhere_context_conn(conn)?;
                        conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [id], |r| r.get(0))?
                    };
                    let mut copy = original.clone();
                    if let Some(payload) = copy.payload.as_mut() { payload["context_uuid"] = serde_json::Value::String(target); }
                    normalized = Some(copy);
                }
            }
        }
        let op = normalized.as_ref().unwrap_or(original);
        let (applied, touched) = match op.table.as_str() {
            "dictionary" | NATURAL_KEY_TOMBSTONE_TABLE => {
                (apply_dictionary_op(conn, op)?, &mut summary.dictionary)
            }
            "snippets" => (apply_snippet_op(conn, op)?, &mut summary.snippets),
            "contexts" => (apply_context_op(conn, op)?, &mut summary.contexts),
            "context_sub_apps" => {
                (apply_sub_app_op(conn, op)?, &mut summary.contexts)
            }
            "dictionary_corrections" => {
                let applied = apply_dictionary_correction_op(conn, op)?;
                if applied == Applied::Yes {
                    summary.dictionary = true;
                }
                (applied, &mut summary.dictionary_corrections)
            }
            "transcriptions" | "held_transcriptions" | "held_hourly_transcriptions" => (apply_history_with_retention(conn, op, cutoff)?, &mut summary.history),
            "api_calls" => (apply_api_call_op(conn, op)?, &mut summary.history),
            "held_api_calls" => (apply_api_call_op(conn, op)?, &mut summary.history),
            "transcription_receipts" | "call_receipts" => (apply_receipt_op(conn, op)?, &mut summary.history),
            other => {
                log::warn!("sync: ignoring op for unknown table {other:?}");
                summary.skipped += 1;
                continue;
            }
        };
        match applied {
            Applied::Yes => {
                if matches!(original.table.as_str(), "transcriptions" | "held_transcriptions" | "held_hourly_transcriptions" | "transcription_receipts") && !original.is_delete() {
                    if let Some(source) = original.payload.as_ref().and_then(|p| p.get("context_uuid")).and_then(|v| v.as_str()) {
                        conn.execute("INSERT OR IGNORE INTO backup_context_event_sources(transcription_uuid,source_uuid,local_uuid,source_name) SELECT ?1,a.source_uuid,a.local_uuid,a.source_name FROM backup_context_aliases a JOIN contexts c ON c.uuid=a.local_uuid WHERE a.source_uuid=?2 AND (EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1 AND context_id=c.id) OR EXISTS(SELECT 1 FROM transcription_receipts WHERE uuid=?1 AND context_uuid=c.uuid))", params![original.row_uuid,source])?;
                    }
                }
                if original.table == "call_receipts" && !original.is_delete() {
                    if let (Some(parent), Some(source)) = (original.payload.as_ref().and_then(|p| p.get("transcription_uuid")).and_then(|v| v.as_str()), original.payload.as_ref().and_then(|p| p.get("context_uuid")).and_then(|v| v.as_str())) {
                        conn.execute("INSERT OR IGNORE INTO backup_context_event_sources(transcription_uuid,source_uuid,local_uuid,source_name) SELECT ?1,a.source_uuid,a.local_uuid,a.source_name FROM backup_context_aliases a JOIN call_receipts r ON r.context_uuid=a.local_uuid WHERE a.source_uuid=?2 AND r.uuid=?3 AND r.transcription_uuid=?1", params![parent,source,original.row_uuid])?;
                    }
                }
                *touched = true;
                summary.applied += 1;
            }
            Applied::Skipped => summary.skipped += 1,
            Applied::Deferred => {
                summary.deferred = true;
                summary.skipped += 1;
            }
        }
    }
    Ok(summary)
}

fn log_applied(conn: &Connection, op: &SyncOp) -> Result<()> {
    sync_store::append_op(conn, op, op.ts_ms, &op.origin, op.origin_seq)
}

fn log_anti_entropy_delete(conn: &Connection, table: &str, row_uuid: &str) -> Result<()> {
    append_self_log(conn, table, row_uuid, "delete")
}

fn log_natural_key_delete(conn: &Connection, row_uuid: &str) -> Result<()> {
    append_self_log(conn, NATURAL_KEY_TOMBSTONE_TABLE, row_uuid, "delete")
}

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

fn op_payload<'a, T: Deserialize<'a>>(
    op: &'a SyncOp,
    missing: &'static str,
    invalid: &'static str,
) -> Result<T> {
    T::deserialize(
        op.payload
            .as_ref()
            .ok_or_else(|| anyhow!("{missing} upsert missing payload"))?,
    )
    .with_context(|| format!("invalid {invalid} payload"))
}

fn apply_dictionary_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.table == NATURAL_KEY_TOMBSTONE_TABLE {
        return apply_dictionary_natural_key_delete(conn, op);
    }
    if op.is_delete() {
        return apply_simple_delete(conn, op, "dictionary", "dictionary");
    }
    if let Some(stamp) = latest_dictionary_stamp(conn, &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    let row: DictionaryRow = op_payload(op, "dictionary", "dictionary")?;

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
    let row: DictionaryCorrectionRow =
        op_payload(op, "dictionary correction", "dictionary correction")?;

    let Some(context_id) = conn
        .query_row(
            "SELECT id FROM contexts WHERE uuid = ?1",
            params![row.context_uuid],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        if latest_is_delete(conn, "contexts", &row.context_uuid)? {
            log_anti_entropy_delete(conn, "dictionary_corrections", &op.row_uuid)?;
            return Ok(Applied::Skipped);
        }
        return Ok(Applied::Deferred);
    };
    let dictionary_is_deleted = latest_is_delete(conn, "dictionary", &row.dictionary_uuid)?;
    if dictionary_is_deleted {
        log_anti_entropy_delete(conn, "dictionary_corrections", &op.row_uuid)?;
        return Ok(Applied::Skipped);
    }
    let Some(dictionary_id) = resolve_correction_dictionary_id(conn, &row)? else {
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
                log_anti_entropy_delete(conn, "dictionary_corrections", &op.row_uuid)?;
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
            log_anti_entropy_delete(conn, "dictionary_corrections", &op.row_uuid)?;
            return Ok(Applied::Skipped);
        }
        delete_local_correction_for_remote_winner(conn, conflict.0, conflict.1.as_deref())?;
    }

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

fn apply_sub_app_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() {
        return apply_simple_delete(conn, op, "context_sub_apps", "context_sub_apps");
    }
    if let Some(stamp) = latest_stamp(conn, "context_sub_apps", &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    let row: SubAppRow = op_payload(op, "sub-app", "sub-app")?;
    let mode = db::TitleMatchMode::parse(&row.match_mode)?;
    let executable = row.executable.trim().to_lowercase();
    let pattern = row.title_pattern.trim();
    if executable.is_empty() || pattern.is_empty() || row.label.trim().is_empty() {
        anyhow::bail!("sub-app payload is missing its app, rule, or name");
    }
    let context_id: Option<i64> = match row.context_uuid.as_deref() {
        Some(uuid) => conn
            .query_row(
                "SELECT id FROM contexts WHERE uuid = ?1 AND is_everywhere = 0",
                params![uuid],
                |r| r.get(0),
            )
            .optional()?,
        None => None,
    };
    conn.execute(
        "DELETE FROM context_sub_apps
          WHERE executable = ?1 AND title_pattern = ?2 AND match_mode = ?3 AND uuid <> ?4",
        params![executable, pattern, mode.as_str(), op.row_uuid],
    )?;
    conn.execute(
        "INSERT INTO context_sub_apps
           (uuid, context_id, executable, app_name, label, icon, title_pattern, match_mode, platform, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(uuid) DO UPDATE SET context_id = excluded.context_id,
             executable = excluded.executable, app_name = excluded.app_name,
             label = excluded.label, icon = excluded.icon,
             title_pattern = excluded.title_pattern, match_mode = excluded.match_mode,
             platform = excluded.platform, updated_at = datetime('now')",
        params![
            op.row_uuid,
            context_id,
            executable,
            row.app_name,
            row.label.trim(),
            row.icon,
            pattern,
            mode.as_str(),
            row.platform,
            row.created_at,
        ],
    )?;
    log_applied(conn, op)?;
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
    let row: SnippetRow = op_payload(op, "snippet", "snippet")?;

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
            let Some(conflict_uuid) = find_conflict(conn)? else {
                return Err(anyhow!(
                    "sync: {table} constraint failed but no conflicting row found for {}",
                    op.row_uuid
                ));
            };
            let conflict_stamp =
                latest_stamp(conn, table, &conflict_uuid)?.unwrap_or((0, String::new(), 0));
            if op.newer_than(&conflict_stamp) {
                if table == "dictionary" {
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

fn apply_dictionary_natural_key_delete(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if let Some(stamp) = latest_stamp(conn, NATURAL_KEY_TOMBSTONE_TABLE, &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }

    let children = capture_dictionary_children(conn, &op.row_uuid)?;
    if !children.context_ids.is_empty() || !children.corrections.is_empty() {
        log_applied(conn, op)?;
        return Ok(Applied::Yes);
    }

    apply_simple_delete(conn, op, "dictionary", NATURAL_KEY_TOMBSTONE_TABLE)
}

fn learn_everywhere_alias(conn: &Connection, uuid: &str) -> Result<bool> {
    let conflicts: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM contexts WHERE uuid=?1 AND is_everywhere=0)", [uuid], |r| r.get(0))?;
    anyhow::ensure!(!conflicts, "Everywhere identity conflicts with a local Context");
    let id = db::ensure_everywhere_context_conn(conn)?;
    let local_uuid: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [id], |r| r.get(0))?;
    let inserted = conn.execute("INSERT OR IGNORE INTO sync_everywhere_aliases(uuid) VALUES (?1)", [uuid])?;
    let pending: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts WHERE context_uuid=?1)", [uuid], |r| r.get(0))?;
    db::resolve_pending_contexts(conn, uuid, id, &local_uuid)?;
    Ok(inserted > 0 || pending)
}

fn apply_context_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() {
        return apply_context_delete(conn, op);
    }
    let aggregate: ContextAggregate = op_payload(op, "context", "context")?;
    let resolved_alias = if aggregate.is_everywhere { learn_everywhere_alias(conn, &op.row_uuid)? } else { false };
    if let Some(stamp) = latest_stamp(conn, "contexts", &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(if resolved_alias { Applied::Yes } else { Applied::Skipped });
        }
    }
    if context_has_missing_members(conn, &aggregate)? {
        return Ok(Applied::Deferred);
    }

    if aggregate.is_everywhere {
        apply_everywhere_aggregate(conn, &aggregate)?;
        log_applied(conn, op)?;
        return Ok(Applied::Yes);
    }

    let alias: Option<(i64, String, Option<String>)> = conn.query_row("SELECT c.id,c.uuid,a.source_name FROM backup_context_aliases a JOIN contexts c ON c.uuid=a.local_uuid WHERE a.source_uuid=?1 AND NOT EXISTS(SELECT 1 FROM contexts WHERE uuid=?1)", [&op.row_uuid], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    if let Some((id, target, original_name)) = alias {
        if original_name.as_deref().is_none_or(|name| name.eq_ignore_ascii_case(aggregate.name.trim())) {
            db::resolve_pending_contexts(conn, &op.row_uuid, id, &target)?;
            log_anti_entropy_delete(conn, "contexts", &op.row_uuid)?;
            return Ok(Applied::Yes);
        }
        conn.execute("DELETE FROM backup_context_aliases WHERE source_uuid=?1", [&op.row_uuid])?;
    }

    let formatting_override = aggregate
        .contextual_formatting_override
        .or_else(|| aggregate.contextual_formatting_disabled.then_some(true));
    let insert = |conn: &Connection| -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO contexts (uuid, name, is_everywhere, icon, tone, cleanup_intensity, color,
                                   custom_instructions, contextual_formatting_disabled, contextual_formatting_override, pinned_at,
                                   created_at, updated_at, paste_in_chunks, t3_skill_mentions_disabled)
             VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, COALESCE(?14, 0))
             ON CONFLICT(uuid) DO UPDATE SET
               name = excluded.name, icon = excluded.icon, tone = excluded.tone,
               cleanup_intensity = excluded.cleanup_intensity, color = excluded.color,
               custom_instructions = excluded.custom_instructions,
               contextual_formatting_disabled = excluded.contextual_formatting_disabled,
               contextual_formatting_override = excluded.contextual_formatting_override,
               pinned_at = excluded.pinned_at, updated_at = excluded.updated_at,
               paste_in_chunks = excluded.paste_in_chunks,
               t3_skill_mentions_disabled = COALESCE(?14, contexts.t3_skill_mentions_disabled)",
            params![
                op.row_uuid,
                aggregate.name,
                aggregate.icon,
                aggregate.tone,
                aggregate.cleanup_intensity,
                aggregate.color,
                aggregate.custom_instructions,
                (formatting_override == Some(true)) as i64,
                formatting_override.map(i64::from),
                aggregate.pinned_at,
                aggregate.created_at,
                aggregate.updated_at,
                aggregate.paste_in_chunks,
                aggregate.t3_skill_mentions_disabled
            ],
        )
    };
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
            let alias: Option<(i64, String)> = conn.query_row("SELECT c.id,c.uuid FROM backup_context_aliases a JOIN contexts c ON c.uuid=a.local_uuid WHERE a.source_uuid=?1 AND c.uuid=?2 AND NOT EXISTS(SELECT 1 FROM contexts WHERE uuid=?1)", params![op.row_uuid,conflict_uuid], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((id, uuid)) = alias {
                db::resolve_pending_contexts(conn, &op.row_uuid, id, &uuid)?;
                log_anti_entropy_delete(conn, "contexts", &op.row_uuid)?;
                return Ok(Applied::Yes);
            } else if op.newer_than(&conflict_stamp) {
                delete_context_by_uuid(conn, &conflict_uuid)?;
                insert(conn).map_err(|e| anyhow!("sync: context retry insert failed: {e}"))?;
            } else {
                delete_context_by_uuid(conn, &op.row_uuid)?;
                log_anti_entropy_delete(conn, "contexts", &op.row_uuid)?;
                return Ok(Applied::Yes);
            }
        }
        Err(err) => return Err(anyhow!("sync: context upsert failed: {err}")),
    }

    let context_id: i64 = conn.query_row(
        "SELECT id FROM contexts WHERE uuid = ?1",
        params![op.row_uuid],
        |r| r.get(0),
    )?;
    let pending: Vec<String> = {
        let mut stmt = conn.prepare("SELECT transcription_uuid FROM pending_transcription_contexts WHERE context_uuid = ?1")?;
        let rows = stmt.query_map([&op.row_uuid], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for uuid in pending {
        db::resolve_pending_context(conn, &uuid, context_id, &op.row_uuid)?;
    }
    reconcile_context_children(conn, context_id, &aggregate)?;
    log_applied(conn, op)?;
    Ok(Applied::Yes)
}

fn apply_everywhere_aggregate(conn: &Connection, aggregate: &ContextAggregate) -> Result<()> {
    let everywhere_id = db::ensure_everywhere_context_conn(conn)?;
    let formatting_override = aggregate
        .contextual_formatting_override
        .or_else(|| aggregate.contextual_formatting_disabled.then_some(true));
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
                custom_instructions = ?5, contextual_formatting_disabled = ?6, contextual_formatting_override = ?7, pinned_at = ?8, paste_in_chunks = ?10, t3_skill_mentions_disabled = COALESCE(?11, t3_skill_mentions_disabled)
         WHERE id = ?9",
        params![
            aggregate.icon,
            aggregate.tone,
            aggregate.cleanup_intensity,
            aggregate.color,
            aggregate.custom_instructions,
            (formatting_override == Some(true)) as i64,
            formatting_override.map(i64::from),
            aggregate.pinned_at,
            everywhere_id,
            aggregate.paste_in_chunks,
            aggregate.t3_skill_mentions_disabled
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
        let alias: Option<(i64, String)> = conn.query_row("SELECT c.id,c.uuid FROM backup_context_aliases a JOIN contexts c ON c.uuid=a.local_uuid WHERE a.source_uuid=?1", [uuid], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((id, target)) = alias {
            db::resolve_pending_contexts(conn, uuid, id, &target)?;
            return Ok(());
        }
        let everywhere_id = db::ensure_everywhere_context_conn(conn)?;
        let everywhere_uuid: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [everywhere_id], |r| r.get(0))?;
        db::resolve_pending_contexts(conn, uuid, everywhere_id, &everywhere_uuid)?;
        return Ok(());
    };
    let is_everywhere: bool = conn.query_row(
        "SELECT is_everywhere != 0 FROM contexts WHERE id = ?1",
        params![context_id],
        |r| r.get(0),
    )?;
    if is_everywhere {
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
    let has_unresolved_targets = aggregate
        .targets
        .iter()
        .any(|entry| entry.executable().starts_with(UNRESOLVED_APP_PREFIX));
    let my_platform = db::current_platform_tag();
    let sticky_executables: std::collections::HashSet<String> = conn
        .prepare(
            "SELECT executable FROM context_targets
               WHERE context_id = ?1
                 AND (platform = ?2 OR (?2 IS NULL AND platform IS NULL))
                 AND executable NOT LIKE '?::%' AND ?3",
        )?
        .query_map(
            params![context_id, my_platform, has_unresolved_targets],
            |r| r.get::<_, String>(0),
        )?
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

fn context_has_missing_members(conn: &Connection, aggregate: &ContextAggregate) -> Result<bool> {
    for (uuid, term) in context_dictionary_references(aggregate) {
        let exists: bool = conn
            .prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM dictionary WHERE uuid = ?1 OR (?2 <> '' AND term = ?2))",
            )?
            .query_row(params![uuid, term.trim()], |r| r.get(0))?;
        if !exists
            && !latest_is_delete(conn, "dictionary", uuid)?
            && !latest_is_delete(conn, NATURAL_KEY_TOMBSTONE_TABLE, uuid)?
        {
            return Ok(true);
        }
    }
    for uuid in &aggregate.snippet_uuids {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM snippets WHERE uuid = ?1)",
            params![uuid],
            |r| r.get(0),
        )?;
        if !exists && !latest_is_delete(conn, "snippets", uuid)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn context_dictionary_references(
    aggregate: &ContextAggregate,
) -> impl Iterator<Item = (&str, &str)> {
    let legacy_uuids = if aggregate.dictionary_entries.is_empty() {
        aggregate.dictionary_uuids.as_slice()
    } else {
        &[]
    };
    aggregate
        .dictionary_entries
        .iter()
        .map(|entry| (entry.uuid.as_str(), entry.term.as_str()))
        .chain(legacy_uuids.iter().map(|uuid| (uuid.as_str(), "")))
}

fn reconcile_context_members(
    conn: &Connection,
    context_id: i64,
    aggregate: &ContextAggregate,
) -> Result<()> {
    let entry_count = if aggregate.dictionary_entries.is_empty() {
        aggregate.dictionary_uuids.len()
    } else {
        aggregate.dictionary_entries.len()
    };
    let mut resolved_dictionary: Vec<i64> = Vec::with_capacity(entry_count);
    let mut unresolved_dictionary = false;
    {
        let mut stmt = conn.prepare_cached("SELECT id FROM dictionary WHERE uuid = ?1")?;
        for (uuid, term) in context_dictionary_references(aggregate) {
            let by_uuid = stmt
                .query_row(params![uuid], |r| r.get::<_, i64>(0))
                .optional()?;
            let by_term = if by_uuid.is_none() && !term.trim().is_empty() {
                conn.prepare_cached("SELECT id FROM dictionary WHERE term = ?1")?
                    .query_row(params![term.trim()], |r| r.get::<_, i64>(0))
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

fn awaiting_pending_context(conn: &Connection, parent: &str, context: Option<&str>) -> Result<bool> {
    let Some(context) = context else { return Ok(false); };
    Ok(conn.query_row("SELECT (NOT EXISTS(SELECT 1 FROM contexts WHERE uuid=?2) OR EXISTS(SELECT 1 FROM contexts WHERE uuid=?2 AND is_everywhere=1)) AND (EXISTS(SELECT 1 FROM pending_transcription_contexts WHERE transcription_uuid=?1 AND context_uuid<>?2) OR EXISTS(SELECT 1 FROM transcriptions t LEFT JOIN contexts c ON c.id=t.context_id WHERE t.uuid=?1 AND c.uuid IS NOT ?2) OR EXISTS(SELECT 1 FROM transcription_receipts WHERE uuid=?1 AND context_uuid IS NOT ?2) OR EXISTS(SELECT 1 FROM call_receipts WHERE transcription_uuid=?1 AND context_uuid IS NOT ?2))", params![parent,context], |r| r.get(0))?)
}

fn apply_receipt_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() { return Ok(Applied::Skipped); }
    let payload = op.payload.as_ref().ok_or_else(|| anyhow!("Missing receipt payload"))?;
    anyhow::ensure!(payload.get("uuid").and_then(|v| v.as_str()) == Some(&op.row_uuid), "Receipt identity mismatch");
    let parent = if op.table == "call_receipts" { payload.get("transcription_uuid").and_then(|v| v.as_str()) } else { Some(op.row_uuid.as_str()) };
    if let Some(parent) = parent {
        if let (Some(target), Some(sources)) = (payload.get("context_uuid").and_then(|v| v.as_str()), payload.get("context_sources").and_then(|v| v.as_array())) {
            let pending: Option<String> = conn.query_row("SELECT context_uuid FROM pending_transcription_contexts WHERE transcription_uuid=?1", [parent], |r| r.get(0)).optional()?;
            if let Some(source) = pending.filter(|source| sources.iter().any(|value| value.as_str() == Some(source))) {
                let target_id: Option<i64> = conn.query_row("SELECT id FROM contexts WHERE uuid=?1", [target], |r| r.get(0)).optional()?;
                let matching = if op.table == "transcription_receipts" {
                    conn.query_row("SELECT EXISTS(SELECT 1 FROM transcription_receipts WHERE uuid=?1 AND day=?2 AND hour=?3 AND words=?4 AND spoken_words IS ?5 AND duration_ms=?6 UNION ALL SELECT 1 FROM transcriptions WHERE uuid=?1 AND activity_day=?2 AND activity_hour=?3 AND words=?4 AND spoken_words IS ?5 AND duration_ms=?6)", params![op.row_uuid,payload.get("day").and_then(|v| v.as_str()),payload.get("hour").and_then(|v| v.as_i64()),payload.get("words").and_then(|v| v.as_i64()),payload.get("spoken_words").and_then(|v| v.as_i64()),payload.get("duration_ms").and_then(|v| v.as_i64())], |r| r.get::<_,bool>(0))?
                } else {
                    conn.query_row("SELECT EXISTS(SELECT 1 FROM call_receipts WHERE uuid=?1 AND transcription_uuid=?2 AND day=?3 AND model=?4 AND provider=?5 AND task=?6 AND audio_ms=?7 AND input_chars=?8 AND output_chars=?9 UNION ALL SELECT 1 FROM api_calls a JOIN transcriptions t ON t.id=a.transcription_id WHERE a.uuid=?1 AND t.uuid=?2 AND a.activity_day=?3 AND a.model=?4 AND a.provider=?5 AND a.task=?6 AND a.audio_ms=?7 AND a.input_chars=?8 AND a.output_chars=?9)", params![op.row_uuid,parent,payload.get("day").and_then(|v| v.as_str()),payload.get("model").and_then(|v| v.as_str()),payload.get("provider").and_then(|v| v.as_str()),payload.get("task").and_then(|v| v.as_str()),payload.get("audio_ms").and_then(|v| v.as_i64()),payload.get("input_chars").and_then(|v| v.as_i64()),payload.get("output_chars").and_then(|v| v.as_i64())], |r| r.get::<_,bool>(0))?
                };
                if let (Some(id), true) = (target_id, matching) {
                    db::resolve_pending_context(conn, parent, id, target)?;
                    conn.execute("INSERT OR IGNORE INTO backup_context_event_sources(transcription_uuid,source_uuid,local_uuid) VALUES (?1,?2,?3)", params![parent,source,target])?;
                }
            }
        }
        if awaiting_pending_context(conn, parent, payload.get("context_uuid").and_then(|v| v.as_str()))? { return Ok(Applied::Deferred); }
    }
    if op.table == "transcription_receipts" {
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1 UNION ALL SELECT 1 FROM transcription_receipts WHERE uuid=?1)", [&op.row_uuid], |r| r.get(0))?;
        if !exists && db::has_conflicting_call_context(conn, &op.row_uuid, payload.get("context_uuid").and_then(|v| v.as_str()))? {
            conn.execute("INSERT OR IGNORE INTO sync_rejected_history VALUES (?1)", [&op.row_uuid])?;
            log::warn!("sync: compacted history conflicts with an existing provider call Context");
            return Ok(Applied::Skipped);
        }
    }
    let exists: bool = conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {} WHERE uuid=?1)", op.table), [&op.row_uuid], |r| r.get(0))?;
    let conflicts = crate::commands::settings::import_sync_receipt(conn, &op.table, payload.clone())?;
    if conflicts > 0 { log::warn!("sync: compacted activity has an ambiguous or conflicting contribution"); }
    if exists { return Ok(if conflicts == 0 { Applied::Yes } else { Applied::Skipped }); }
    let inserted: bool = conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {} WHERE uuid=?1)", op.table), [&op.row_uuid], |r| r.get(0))?;
    if inserted {
        if op.table == "transcription_receipts" { conn.execute("DELETE FROM sync_rejected_history WHERE uuid=?1", [&op.row_uuid])?; }
        log_applied(conn, op)?;
        Ok(Applied::Yes)
    } else {
        Ok(Applied::Skipped)
    }
}

fn resolve_replayed_receipt(conn: &Connection, op: &SyncOp) -> Result<bool> {
    let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcription_receipts WHERE uuid=?1)", [&op.row_uuid], |r| r.get(0))?;
    if !exists { return Ok(false); }
    let row: TranscriptionRow = op_payload(op, "transcription", "transcription")?;
    let matched: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcription_receipts r JOIN pending_transcription_contexts p ON p.transcription_uuid=r.uuid WHERE r.uuid=?1 AND p.context_uuid IS ?2 AND r.words=?3 AND r.spoken_words IS ?4 AND r.duration_ms=?5 AND r.day=COALESCE(?6,date(?8,'localtime')) AND r.hour=COALESCE(?7,CAST(strftime('%H',?8,'localtime') AS INTEGER)))", params![op.row_uuid,row.context_uuid,row.words,row.spoken_words,row.duration_ms,row.activity_day,row.activity_hour,row.created_at], |r| r.get(0))?;
    if matched {
        if let Some(uuid) = row.context_uuid.as_deref() {
            let id: Option<i64> = conn.query_row("SELECT id FROM contexts WHERE uuid=?1", [uuid], |r| r.get(0)).optional()?;
            if let Some(id) = id { db::resolve_pending_context(conn, &op.row_uuid, id, uuid)?; }
        }
    } else if conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts WHERE transcription_uuid=?1)", [&op.row_uuid], |r| r.get::<_, bool>(0))? {
        log::warn!("sync: retained history does not match its pending compacted receipt");
    }
    Ok(true)
}

fn apply_history_with_retention(conn: &Connection, op: &SyncOp, cutoff: Option<&str>) -> Result<Applied> {
    if op.is_delete() { return Ok(Applied::Skipped); }
    if let Some(payload) = op.payload.as_ref() {
        if let (Some(target), Some(sources)) = (payload.get("context_uuid").and_then(|v| v.as_str()), payload.get("context_sources").and_then(|v| v.as_array())) {
            let pending: Option<String> = conn.query_row("SELECT context_uuid FROM pending_transcription_contexts WHERE transcription_uuid=?1", [&op.row_uuid], |r| r.get(0)).optional()?;
            if let Some(source) = pending.filter(|source| sources.iter().any(|value| value.as_str() == Some(source))) {
                let row: TranscriptionRow = op_payload(op, "transcription", "transcription")?;
                let id: Option<i64> = conn.query_row("SELECT id FROM contexts WHERE uuid=?1", [target], |r| r.get(0)).optional()?;
                let matched: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1 AND raw_text=?2 AND clean_text=?3 AND words=?4 AND spoken_words IS ?5 AND duration_ms=?6 AND api_used=?7 AND app_name IS ?8 AND created_at=?9 AND (?10 IS NULL OR activity_day=?10) AND (?11 IS NULL OR activity_hour=?11) UNION ALL SELECT 1 FROM transcription_receipts WHERE uuid=?1 AND words=?4 AND spoken_words IS ?5 AND duration_ms=?6 AND day=COALESCE(?10,date(?9,'localtime')) AND hour=COALESCE(?11,CAST(strftime('%H',?9,'localtime') AS INTEGER)))", params![op.row_uuid,row.raw_text,row.clean_text,row.words,row.spoken_words,row.duration_ms,row.api_used,row.app_name,row.created_at,row.activity_day,row.activity_hour], |r| r.get(0))?;
                if let (Some(id), true) = (id, matched) {
                    db::resolve_pending_context(conn, &op.row_uuid, id, target)?;
                    conn.execute("INSERT OR IGNORE INTO backup_context_event_sources(transcription_uuid,source_uuid,local_uuid) VALUES (?1,?2,?3)", params![op.row_uuid,source,target])?;
                }
            }
        }
    }
    if awaiting_pending_context(conn, &op.row_uuid, op.payload.as_ref().and_then(|p| p.get("context_uuid")).and_then(|v| v.as_str()))? { return Ok(Applied::Deferred); }
    if resolve_replayed_receipt(conn, op)? { return Ok(Applied::Skipped); }
    if let Some(cutoff) = cutoff {
        let row: TranscriptionRow = op_payload(op, "transcription", "transcription")?;
        let expired: bool = conn.query_row("SELECT datetime(?1) < datetime(?2)", params![row.created_at, cutoff], |r| r.get(0))?;
        if expired {
            let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1 UNION ALL SELECT 1 FROM transcription_receipts WHERE uuid=?1)", [&op.row_uuid], |r| r.get(0))?;
            if exists { return Ok(Applied::Skipped); }
            let (day, hour): (String, i64) = conn.query_row("SELECT COALESCE(?1, date(?3, 'localtime')), COALESCE(?2, CAST(strftime('%H', ?3, 'localtime') AS INTEGER))", params![row.activity_day, row.activity_hour, row.created_at], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let counted = op.table != "held_transcriptions";
            let payload = serde_json::json!({"uuid":op.row_uuid, "day":day, "hour":hour, "context_uuid":row.context_uuid, "words":row.words, "spoken_words":row.spoken_words, "duration_ms":row.duration_ms, "counted":counted, "hourly_counted":op.table != "held_hourly_transcriptions"});
            let conflicts = crate::commands::settings::import_sync_receipt(conn, "transcription_receipts", payload)?;
            if conflicts > 0 { log::warn!("sync: expired history has an uncertain historical summary overlap"); }
            let counted: Option<bool> = conn.query_row("SELECT counted FROM transcription_receipts WHERE uuid=?1", [&op.row_uuid], |r| r.get(0)).optional()?;
            let Some(counted) = counted else {
                if conflicts > 0 { conn.execute("INSERT OR IGNORE INTO sync_rejected_history VALUES (?1)", [&op.row_uuid])?; }
                return Ok(Applied::Skipped);
            };
            conn.execute("DELETE FROM sync_rejected_history WHERE uuid=?1", [&op.row_uuid])?;
            if counted { append_self_log(conn, "transcription_receipts", &op.row_uuid, "upsert")?; }
            return Ok(Applied::Yes);
        }
    }
    if op.table == "held_transcriptions" { apply_held_transcription_op(conn, op) } else { apply_transcription_op(conn, op) }
}

fn apply_held_transcription_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() { return Ok(Applied::Skipped); }
    let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1 UNION ALL SELECT 1 FROM transcription_receipts WHERE uuid=?1)", [&op.row_uuid], |r| r.get(0))?;
    if exists { return Ok(Applied::Skipped); }
    conn.execute("INSERT OR IGNORE INTO backup_held_history(uuid) VALUES (?1)", [&op.row_uuid])?;
    let applied = apply_transcription_op(conn, op)?;
    if applied != Applied::Yes {
        conn.execute("DELETE FROM backup_held_history WHERE uuid=?1 AND NOT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1)", [&op.row_uuid])?;
    }
    Ok(applied)
}

fn apply_transcription_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() {
        return Ok(Applied::Skipped);
    }
    if resolve_replayed_receipt(conn, op)? { return Ok(Applied::Skipped); }
    if let Some(stamp) = latest_stamp(conn, "transcriptions", &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    let row: TranscriptionRow = op_payload(op, "transcription", "transcription")?;
    anyhow::ensure!(row.activity_day.as_ref().is_none_or(|day| chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok()) && row.activity_hour.is_none_or(|hour| (0..24).contains(&hour)), "Invalid transcription calendar bucket");
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
    if db::has_conflicting_call_context(conn, &op.row_uuid, row.context_uuid.as_deref())? {
        conn.execute("INSERT OR IGNORE INTO sync_rejected_history VALUES (?1)", [&op.row_uuid])?;
        log::warn!("sync: history conflicts with an existing provider call Context");
        return Ok(Applied::Skipped);
    }
    let pending_context: Option<String> = conn.query_row("SELECT context_uuid FROM pending_transcription_contexts WHERE transcription_uuid=?1", [&op.row_uuid], |r| r.get(0)).optional()?;
    if let Some(pending) = pending_context.as_deref() {
        if row.context_uuid.as_deref() != Some(pending) {
            conn.execute("INSERT OR IGNORE INTO sync_rejected_history VALUES (?1)", [&op.row_uuid])?;
            log::warn!("sync: history has a conflicting pending Context identity");
            return Ok(Applied::Skipped);
        }
        let parent_exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transcriptions WHERE uuid=?1 UNION ALL SELECT 1 FROM transcription_receipts WHERE uuid=?1)", [&op.row_uuid], |r| r.get(0))?;
        if !parent_exists {
            if let Some(id) = context_id { db::resolve_pending_context(conn, &op.row_uuid, id, pending)?; }
        }
    }
    let uncertain: bool = conn.query_row("SELECT NOT EXISTS(SELECT 1 FROM transcriptions WHERE uuid = ?1) AND EXISTS(SELECT 1 FROM legacy_pruned_days WHERE day = COALESCE(?2, date(?3, 'localtime')))", params![op.row_uuid, row.activity_day, row.created_at], |r| r.get(0))?;
    if uncertain {
        conn.execute("INSERT OR IGNORE INTO backup_held_history(uuid) VALUES (?1)", [&op.row_uuid])?;
        log::warn!("sync: retained a transcription with uncertain historical summary overlap");
    }
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO transcriptions
           (uuid, raw_text, clean_text, words, spoken_words, duration_ms, api_used, app_name, context_id, created_at, activity_day, activity_hour)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
            row.created_at,
            row.activity_day,
            row.activity_hour
        ],
    )?;
    if inserted > 0 {
        conn.execute("DELETE FROM sync_rejected_history WHERE uuid=?1", [&op.row_uuid])?;
        if op.table == "held_hourly_transcriptions" {
            conn.execute("INSERT OR IGNORE INTO backup_held_hourly_receipts(uuid) VALUES (?1)", [&op.row_uuid])?;
        }
        if !uncertain && (context_id.is_some() || row.context_uuid.is_none()) && db::hold_ambiguous_retained_hour(conn, &op.row_uuid, op.table == "held_hourly_transcriptions")? {
            log::warn!("sync: retained a transcription with uncertain hourly overlap");
        }
        if context_id.is_none() {
            if let Some(context_uuid) = row.context_uuid.as_deref() {
                conn.execute(
                    "INSERT OR REPLACE INTO pending_transcription_contexts
                       (transcription_uuid, context_uuid) VALUES (?1, ?2)",
                    params![op.row_uuid, context_uuid],
                )?;
            }
        }
        log_applied(conn, op)?;
        Ok(Applied::Yes)
    } else {
        Ok(Applied::Skipped)
    }
}

fn apply_api_call_op(conn: &Connection, op: &SyncOp) -> Result<Applied> {
    if op.is_delete() {
        return apply_simple_delete(conn, op, "api_calls", "api_calls");
    }
    if conn.query_row("SELECT EXISTS(SELECT 1 FROM call_receipts WHERE uuid = ?1)", [&op.row_uuid], |r| r.get::<_, bool>(0))? {
        return Ok(Applied::Skipped);
    }
    if let Some(stamp) = latest_stamp(conn, "api_calls", &op.row_uuid)? {
        if !op.newer_than(&stamp) {
            return Ok(Applied::Skipped);
        }
    }
    let row: ApiCallRow = op_payload(op, "api_call", "api call")?;
    anyhow::ensure!(row.activity_day.as_ref().is_none_or(|day| chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok()), "Invalid provider calendar bucket");
    anyhow::ensure!(row.audio_ms >= 0 && row.input_chars >= 0 && row.output_chars >= 0, "Invalid provider usage");
    if conn.query_row("SELECT EXISTS(SELECT 1 FROM api_calls WHERE uuid=?1)", [&op.row_uuid], |r| r.get::<_, bool>(0))? {
        log::warn!("sync: retained provider call already exists; preserving the local record");
        return Ok(Applied::Skipped);
    }
    let Some(transcription_uuid) = row.transcription_uuid.as_deref() else {
        let day: String = conn.query_row("SELECT COALESCE(?1, date(?2, 'localtime'))", params![row.activity_day, row.created_at], |r| r.get(0))?;
        let mut call = db::CompactedCall { uuid: &op.row_uuid, transcription_uuid: None, day: &day, context_uuid: None, context_id: None, model: &row.model, provider: &row.provider, task: &row.task, audio_ms: row.audio_ms, input_chars: row.input_chars, output_chars: row.output_chars, counted: op.table != "held_api_calls" };
        call.counted &= !db::has_legacy_provider(conn, &call)?;
        return match db::insert_compacted_call(conn, &call)? {
            db::ReceiptOutcome::Inserted => { if call.counted { append_self_log(conn, "call_receipts", &op.row_uuid, "upsert")?; } Ok(Applied::Yes) },
            db::ReceiptOutcome::Existing => Ok(Applied::Skipped),
            db::ReceiptOutcome::Conflict => anyhow::bail!("Conflicting provider receipt"),
        };
    };
    let Some(transcription_id) = conn
        .query_row(
            "SELECT id FROM transcriptions WHERE uuid = ?1",
            params![transcription_uuid],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        let parent: Option<(Option<String>, bool)> = conn.query_row("SELECT context_uuid, counted FROM transcription_receipts WHERE uuid = ?1", [transcription_uuid], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if let Some((context_uuid, _parent_counted)) = parent {
            let day: String = conn.query_row("SELECT COALESCE(?1, date(?2, 'localtime'))", params![row.activity_day, row.created_at], |r| r.get(0))?;
            let context_id: Option<i64> = conn.query_row("SELECT id FROM contexts WHERE uuid = ?1", [context_uuid.as_deref()], |r| r.get(0)).optional()?;
            if let (Some(id), Some(uuid)) = (context_id, context_uuid.as_deref()) {
                let pending: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts WHERE transcription_uuid=?1 AND context_uuid=?2)", params![transcription_uuid,uuid], |r| r.get(0))?;
                if pending { db::resolve_pending_context(conn, transcription_uuid, id, uuid)?; }
            }
            let effective_context = if context_id.is_some() { context_uuid.as_deref() } else { None };
            let mut call = db::CompactedCall { uuid: &op.row_uuid, transcription_uuid: Some(transcription_uuid), day: &day, context_uuid: effective_context, context_id, model: &row.model, provider: &row.provider, task: &row.task, audio_ms: row.audio_ms, input_chars: row.input_chars, output_chars: row.output_chars, counted: op.table != "held_api_calls" };
            call.counted &= !db::has_legacy_provider(conn, &call)?;
            call.context_uuid = context_uuid.as_deref();
            match db::insert_compacted_call(conn, &call)? {
                db::ReceiptOutcome::Inserted => { if call.counted { append_self_log(conn, "call_receipts", &op.row_uuid, "upsert")?; } return Ok(Applied::Yes); },
                db::ReceiptOutcome::Existing => return Ok(Applied::Skipped),
                db::ReceiptOutcome::Conflict => anyhow::bail!("Conflicting provider receipt"),
            }
        }
        let rejected: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sync_rejected_history WHERE uuid=?1)", [transcription_uuid], |r| r.get(0))?;
        if rejected {
            log::warn!("sync: skipping provider call for rejected history");
            return Ok(Applied::Skipped);
        }
        return Ok(Applied::Deferred);
    };
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO api_calls
           (uuid, transcription_id, model, provider, task, audio_ms, input_chars, output_chars, created_at, activity_day)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            op.row_uuid,
            transcription_id,
            row.model,
            row.provider,
            row.task,
            row.audio_ms,
            row.input_chars,
            row.output_chars,
            row.created_at,
            row.activity_day
        ],
    )?;
    if inserted > 0 {
        let pending: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pending_transcription_contexts WHERE transcription_uuid = ?1)", [transcription_uuid], |r| r.get(0))?;
        if pending && op.table == "held_api_calls" {
            conn.execute("INSERT OR IGNORE INTO backup_held_calls(uuid) VALUES (?1)", [&op.row_uuid])?;
        }
        if !pending && db::hold_ambiguous_retained_call(conn, &op.row_uuid, op.table == "held_api_calls")? {
            log::warn!("sync: retained a provider call with uncertain historical usage overlap");
        }
        log_applied(conn, op)?;
        Ok(Applied::Yes)
    } else {
        Ok(Applied::Skipped)
    }
}

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
        let _ = peer_uuid;
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
        return Err(anyhow!("failed to persist synced settings: {err}"));
    }
    for record in accepted {
        sync_store::set_setting_stamp(conn, &record.key, record.ts_ms, &record.origin)?;
    }
    Ok(values.len())
}

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
        activity_receipts: true,
    }
}

fn note_receipt_capability(conn: &Connection, peer: &str, supported: bool) -> Result<()> {
    let previous: Option<bool> = conn.query_row("SELECT activity_receipts FROM sync_peer_capabilities WHERE device_uuid=?1", [peer], |r| r.get(0)).optional()?;
    if supported && previous != Some(true) {
        conn.execute("UPDATE sync_peers SET needs_snapshot=1 WHERE device_uuid=?1", [peer])?;
    }
    conn.execute("INSERT INTO sync_peer_capabilities(device_uuid, activity_receipts) VALUES (?1, ?2) ON CONFLICT(device_uuid) DO UPDATE SET activity_receipts=excluded.activity_receipts", params![peer, supported])?;
    Ok(())
}

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

    let remote = if initiator {
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
            Message::HelloAck(remote) => { check_hello(&remote, &peer.device_uuid)?; remote },
            Message::Error { message } => return Err(anyhow!("peer error: {message}")),
            _ => return Err(anyhow!("expected HelloAck")),
        }
    } else {
        let remote = if let Some(remote) = remote_hello {
            check_hello(&remote, &peer.device_uuid)?;
            remote
        } else {
            match read_message(stream).await? {
                Message::Hello(remote) => { check_hello(&remote, &peer.device_uuid)?; remote },
                Message::Error { message } => return Err(anyhow!("peer error: {message}")),
                _ => return Err(anyhow!("expected Hello")),
            }
        };
        send_message(
            stream,
            &Message::HelloAck(hello(
                &host.device_uuid(),
                &host.device_name(),
                &host.app_version(),
            )),
        )
        .await?;
        remote
    };
    {
        let conn = lock(db)?;
        note_receipt_capability(&conn, &peer.device_uuid, remote.activity_receipts)?;
    }

    let mut summary = SessionSummary::default();

    let stats = {
        let conn = lock(db)?;
        build_stats_exchange(&conn, &host.device_uuid())?
    };
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
        _ => return Err(anyhow!("expected Meta")),
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

    let mut pulled = ApplySummary::default();
    if initiator {
        pulled.merge(pull_from_peer(db, host, stream, peer).await?);
        serve_peer_pulls(db, stream, peer).await?;
    } else {
        serve_peer_pulls(db, stream, peer).await?;
        pulled.merge(pull_from_peer(db, host, stream, peer).await?);
    }
    summary.applied.merge(pulled);

    if summary.applied.deferred {
        return Err(anyhow!(
            "Sync is waiting for referenced data; the next session will retry"
        ));
    }

    send_message(stream, &Message::SyncDone).await?;
    match read_message(stream).await? {
        Message::SyncDone => {}
        _ => return Err(anyhow!("expected sync completion confirmation")),
    }
    Ok(summary)
}

fn check_hello(remote: &Hello, expected_uuid: &str) -> Result<()> {
    if remote.device_uuid != expected_uuid {
        return Err(anyhow!(
            "sync peer identity does not match the paired device"
        ));
    }
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
                let OpsBatch {
                    ops, cursor, done, ..
                } = batch;
                let resolved_ops = resolve_context_targets_in_ops(host, ops);
                let applied = {
                    let conn = lock(db)?;
                    apply_ops_with_retention(&conn, &resolved_ops, host.history_retention_days())?
                };
                summary.merge(applied);
                let acked = cursor;
                send_message(stream, &Message::Ack { seq: acked }).await?;
                if done {
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
            Message::SyncDone => return Err(anyhow!("peer ended sync before completing the pull")),
            Message::Error { message } => return Err(anyhow!("peer error: {message}")),
            _ => return Err(anyhow!("unexpected message during pull")),
        }
    }
    Ok(summary)
}

async fn serve_peer_pulls<S>(db: &DbHandle, stream: &mut S, peer: &SyncPeer) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match super::protocol::read_message(stream).await? {
        Message::PullRequest(request) => {
            serve_one_pull(db, stream, peer, request.since_seq, request.snapshot).await
        }
        Message::SyncDone => Err(anyhow!("peer ended sync before requesting its pull")),
        Message::Error { message } => Err(anyhow!("peer error: {message}")),
        _ => Err(anyhow!("expected PullRequest")),
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
                if seq != final_cursor {
                    return Err(anyhow!("peer acknowledged an unexpected sync cursor"));
                }
                let conn = lock(db)?;
                sync_store::set_peer_send_position(
                    &conn,
                    &peer.device_uuid,
                    seq,
                    snapshot && !done,
                )?;
            }
            Message::Error { message } => return Err(anyhow!("peer error: {message}")),
            _ => return Err(anyhow!("unexpected message during serve")),
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

#[cfg(test)]
mod payload_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pending_history_sync_payload_keeps_context_identity() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',5,1000,'test','2020-01-01 10:00:00')", []).unwrap();
        conn.execute("INSERT INTO pending_transcription_contexts VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')", []).unwrap();
        let payload = transcription_payload(&conn, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap().unwrap();
        assert_eq!(payload["context_uuid"], "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
    }

    #[test]
    fn unresolved_receipts_sync_without_blocking_the_batch() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        let receipt = json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","hour":10,"context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","words":5,"spoken_words":null,"duration_ms":1000});
        crate::commands::settings::import_sync_receipt(&source.lock().unwrap(), "transcription_receipts", receipt).unwrap();
        let (ops, _, _) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        let conn = dest.lock().unwrap();
        assert!(!apply_ops(&conn, &ops).unwrap().deferred);
        assert_eq!(conn.query_row("SELECT context_uuid FROM pending_transcription_contexts", [], |r| r.get::<_, String>(0)).unwrap(), "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn deleting_missing_context_rehomes_pending_activity_once() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        let mut parent = json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","hour":10,"context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","words":5,"spoken_words":null,"duration_ms":1000});
        let mut call = json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", parent.clone()).unwrap();
        crate::commands::settings::import_sync_receipt(&conn, "call_receipts", call.clone()).unwrap();
        let op = SyncOp { table:"contexts".into(), row_uuid:"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(), op:"delete".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:1, payload:None };
        apply_ops(&conn, &[op.clone()]).unwrap();
        apply_ops(&conn, &[op.clone()]).unwrap();
        let everywhere: String = conn.query_row("SELECT uuid FROM contexts WHERE is_everywhere=1", [], |r| r.get(0)).unwrap();
        parent["context_uuid"] = json!(everywhere);
        call["context_uuid"] = parent["context_uuid"].clone();
        assert_eq!(crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", parent.clone()).unwrap(), 0);
        assert_eq!(crate::commands::settings::import_sync_receipt(&conn, "call_receipts", call.clone()).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_hourly_stats WHERE context_id=(SELECT id FROM contexts WHERE is_everywhere=1)", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats WHERE context_id=(SELECT id FROM contexts WHERE is_everywhere=1)", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        let later = db::open(":memory:").unwrap();
        let later_conn = later.lock().unwrap();
        apply_ops(&later_conn, &[op]).unwrap();
        let original_context = json!("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
        parent["context_uuid"] = original_context.clone();
        call["context_uuid"] = original_context;
        let parent_op = SyncOp { table:"transcription_receipts".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:2, payload:Some(parent) };
        let call_op = SyncOp { table:"call_receipts".into(), row_uuid:"cccccccc-cccc-4ccc-8ccc-cccccccccccc".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:3, payload:Some(call) };
        assert!(!apply_ops(&later_conn, &[parent_op, call_op]).unwrap().deferred);
        assert_eq!(later_conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(later_conn.query_row("SELECT SUM(calls) FROM provider_daily_stats WHERE context_id=(SELECT id FROM contexts WHERE is_everywhere=1)", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn resolved_pending_receipts_relay_to_existing_peer() {
      for (sender_retained, retained) in [(false,false),(false,true),(true,false),(true,true)] {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        for (database, identity) in [(&source,"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"),(&dest,"ffffffff-ffff-4fff-8fff-ffffffffffff")] {
            let conn = database.lock().unwrap();
            sync_store::ensure_self_identity(&conn, identity, "test").unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
            if (retained && identity.starts_with('f')) || (sender_retained && identity.starts_with('e')) {
                conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at,activity_day,activity_hour) VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc','a','a',5,1000,'test','2020-01-01 10:00:00','2020-01-01',10)", []).unwrap();
                conn.execute("INSERT INTO pending_transcription_contexts VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc','aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')", []).unwrap();
                conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,audio_ms,input_chars,output_chars,created_at,activity_day) SELECT 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',id,'model','provider','stt',1,0,0,'2020-01-01 10:00:00','2020-01-01' FROM transcriptions WHERE uuid='cccccccc-cccc-4ccc-8ccc-cccccccccccc'", []).unwrap();
            } else {
                crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","hour":10,"context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","words":5,"spoken_words":null,"duration_ms":1000})).unwrap();
                crate::commands::settings::import_sync_receipt(&conn, "call_receipts", json!({"uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","transcription_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":1,"input_chars":0,"output_chars":0})).unwrap();
            }
        }
        let conn = source.lock().unwrap();
        let before = sync_store::max_log_seq(&conn).unwrap();
        conn.execute("INSERT INTO backup_context_aliases(source_uuid,local_uuid,source_name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        let id: i64 = conn.query_row("SELECT id FROM contexts WHERE name='Work'", [], |r| r.get(0)).unwrap();
        db::resolve_pending_contexts(&conn, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", id, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
        let (ops, _, _) = collect_ops(&conn, before, false, 1000, &mut SnapshotProgress::default()).unwrap();
        drop(conn);
        let conn = dest.lock().unwrap();
        assert!(!apply_ops(&conn, &ops).unwrap().deferred);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_context_daily_stats WHERE context_uuid='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'", [], |r| r.get::<_,i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT (SELECT COUNT(*) FROM api_calls)+(SELECT COALESCE(SUM(calls),0) FROM provider_daily_stats)", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
      }
    }

    #[test]
    fn pending_receipt_resolution_records_mapped_context() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        let receipt = json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","hour":10,"context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","words":5,"spoken_words":null,"duration_ms":1000});
        crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", receipt.clone()).unwrap();
        conn.execute("INSERT INTO backup_context_aliases(source_uuid,local_uuid,source_name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        let context = SyncOp { table:"contexts".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1, payload:Some(json!({"name":"Work","created_at":"2020-01-01","updated_at":"2020-01-01"})) };
        apply_ops(&conn, &[context]).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_context_event_sources WHERE transcription_uuid='cccccccc-cccc-4ccc-8ccc-cccccccccccc'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        let deletion = SyncOp { table:"contexts".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"delete".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:2, payload:None };
        apply_ops(&conn, &[deletion]).unwrap();
        let everywhere_id = db::ensure_everywhere_context_conn(&conn).unwrap();
        let everywhere: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [everywhere_id], |r| r.get(0)).unwrap();
        let mut moved = receipt;
        moved["context_uuid"] = json!(everywhere);
        let op = SyncOp { table:"transcription_receipts".into(), row_uuid:"cccccccc-cccc-4ccc-8ccc-cccccccccccc".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:3, payload:Some(moved) };
        assert!(!apply_ops(&conn, &[op]).unwrap().deferred);
    }

    #[test]
    fn matching_existing_call_receipt_records_source_context() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        let mut call = json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","day":"2020-01-01","context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","model":"model","provider":"provider","task":"stt","audio_ms":1,"input_chars":0,"output_chars":0});
        crate::commands::settings::import_sync_receipt(&conn, "call_receipts", call.clone()).unwrap();
        conn.execute("INSERT INTO backup_context_aliases(source_uuid,local_uuid,source_name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Work')", []).unwrap();
        call["context_uuid"] = json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let mut op = SyncOp { table:"call_receipts".into(), row_uuid:"cccccccc-cccc-4ccc-8ccc-cccccccccccc".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1, payload:Some(call) };
        assert!(!apply_ops(&conn, &[op.clone()]).unwrap().deferred);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_context_event_sources WHERE transcription_uuid='dddddddd-dddd-4ddd-8ddd-dddddddddddd'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        let deletion = SyncOp { table:"contexts".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"delete".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:2, payload:None };
        apply_ops(&conn, &[deletion]).unwrap();
        let everywhere_id = db::ensure_everywhere_context_conn(&conn).unwrap();
        let everywhere: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [everywhere_id], |r| r.get(0)).unwrap();
        op.payload.as_mut().unwrap()["context_uuid"] = json!(everywhere);
        assert!(!apply_ops(&conn, &[op]).unwrap().deferred);
    }

    #[test]
    fn rejected_context_collision_resolves_pending_receipts() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Work')", []).unwrap();
        let receipt = json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","day":"2020-01-01","hour":10,"context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","words":5,"spoken_words":null,"duration_ms":1000});
        crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", receipt.clone()).unwrap();
        let context = SyncOp { table:"contexts".into(), row_uuid:"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1, payload:Some(json!({"name":"Work","created_at":"2020-01-01","updated_at":"2020-01-01"})) };
        apply_ops(&conn, &[context]).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        let replay = SyncOp { table:"transcription_receipts".into(), row_uuid:"cccccccc-cccc-4ccc-8ccc-cccccccccccc".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:2, payload:Some(receipt) };
        assert!(!apply_ops(&conn, &[replay]).unwrap().deferred);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_context_daily_stats WHERE context_uuid=(SELECT uuid FROM contexts WHERE is_everywhere=1)", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn synced_provider_overflow_rolls_back() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',1,1000,'test','2020-01-01 10:00:00')", []).unwrap();
        conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,audio_ms,input_chars,output_chars,created_at) SELECT 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',id,'model','provider','stt',?1,0,0,'2020-01-01 10:00:00' FROM transcriptions", [i64::MAX]).unwrap();
        let call = SyncOp { table:"api_calls".into(), row_uuid:"cccccccc-cccc-4ccc-8ccc-cccccccccccc".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1, payload:Some(json!({"transcription_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":1,"input_chars":0,"output_chars":0,"created_at":"2020-01-01 10:00:00"})) };
        assert!(apply_ops(&conn, &[call]).is_err());
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM api_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM sync_log WHERE row_uuid='cccccccc-cccc-4ccc-8ccc-cccccccccccc'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn moved_call_waits_for_known_parent_context_deletion() {
        for retained in [false, true] {
            let db = db::open(":memory:").unwrap();
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Synthetic')", []).unwrap();
            if retained {
                conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,context_id,created_at,activity_day,activity_hour) SELECT 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',5,1000,'test',id,'2020-01-01 10:00:00','2020-01-01',10 FROM contexts WHERE name='Synthetic'", []).unwrap();
            } else {
                crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","hour":10,"context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","words":5,"spoken_words":null,"duration_ms":1000})).unwrap();
            }
            let everywhere_id = db::ensure_everywhere_context_conn(&conn).unwrap();
            let everywhere: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [everywhere_id], |r| r.get(0)).unwrap();
            let call = SyncOp { table:"call_receipts".into(), row_uuid:"cccccccc-cccc-4ccc-8ccc-cccccccccccc".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1, payload:Some(json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","context_uuid":everywhere,"model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0})) };
            assert!(apply_ops(&conn, &[call.clone()]).unwrap().deferred);
            let deletion = SyncOp { table:"contexts".into(), row_uuid:"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(), op:"delete".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:2, payload:None };
            apply_ops(&conn, &[deletion]).unwrap();
            assert!(!apply_ops(&conn, &[call.clone()]).unwrap().deferred);
            assert!(!apply_ops(&conn, &[call]).unwrap().deferred);
            assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        }
    }

    #[test]
    fn moved_call_before_context_tombstone_retries_after_reconciliation() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        let parent = json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","hour":10,"context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","words":5,"spoken_words":null,"duration_ms":1000});
        let first = json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", parent).unwrap();
        crate::commands::settings::import_sync_receipt(&conn, "call_receipts", first).unwrap();
        let second = SyncOp { table:"call_receipts".into(), row_uuid:"dddddddd-dddd-4ddd-8ddd-dddddddddddd".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1, payload:Some(json!({"uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","transcription_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","context_uuid":"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0})) };
        assert!(apply_ops(&conn, &[second.clone()]).unwrap().deferred);
        conn.execute("INSERT INTO sync_everywhere_aliases VALUES ('eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee')", []).unwrap();
        assert!(apply_ops(&conn, &[second.clone()]).unwrap().deferred);
        let deletion = SyncOp { table:"contexts".into(), row_uuid:"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(), op:"delete".into(), ts_ms:2, origin:"peer".into(), origin_seq:2, payload:None };
        apply_ops(&conn, &[deletion]).unwrap();
        assert!(!apply_ops(&conn, &[second]).unwrap().deferred);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }

    #[test]
    fn replayed_everywhere_alias_resolves_existing_pending_activity() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        let (ops, _, _) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        let everywhere = ops.into_iter().find(|op| op.table == "contexts" && op.payload.as_ref().is_some_and(|p| p.get("is_everywhere") == Some(&json!(true)))).unwrap();
        let conn = dest.lock().unwrap();
        apply_ops(&conn, &[everywhere.clone()]).unwrap();
        conn.execute("DELETE FROM sync_everywhere_aliases WHERE uuid=?1", [&everywhere.row_uuid]).unwrap();
        let receipt = json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","hour":10,"context_uuid":everywhere.row_uuid,"words":5,"spoken_words":null,"duration_ms":1000});
        crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", receipt).unwrap();
        apply_ops(&conn, &[everywhere.clone()]).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM sync_everywhere_aliases WHERE uuid=?1", [&everywhere.row_uuid], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_context_daily_stats WHERE context_uuid=(SELECT uuid FROM contexts WHERE is_everywhere=1)", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn moved_parent_before_tombstone_retries_without_losing_history() {
        for retained in [false, true] {
            let db = db::open(":memory:").unwrap();
            let conn = db.lock().unwrap();
            let call = json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","transcription_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","day":"2020-01-01","context_uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
            crate::commands::settings::import_sync_receipt(&conn, "call_receipts", call).unwrap();
            let id = db::ensure_everywhere_context_conn(&conn).unwrap();
            let everywhere: String = conn.query_row("SELECT uuid FROM contexts WHERE id=?1", [id], |r| r.get(0)).unwrap();
            let (table,payload) = if retained {
                ("transcriptions", json!({"raw_text":"a","clean_text":"a","words":5,"duration_ms":1000,"api_used":"test","context_uuid":everywhere,"created_at":"2020-01-01 10:00:00","activity_day":"2020-01-01","activity_hour":10}))
            } else {
                ("transcription_receipts", json!({"uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","day":"2020-01-01","hour":10,"context_uuid":everywhere,"words":5,"spoken_words":null,"duration_ms":1000}))
            };
            let parent = SyncOp { table:table.into(), row_uuid:"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1, payload:Some(payload) };
            assert!(apply_ops(&conn, &[parent.clone()]).unwrap().deferred);
            let deletion = SyncOp { table:"contexts".into(), row_uuid:"cccccccc-cccc-4ccc-8ccc-cccccccccccc".into(), op:"delete".into(), ts_ms:2, origin:"peer".into(), origin_seq:2, payload:None };
            apply_ops(&conn, &[deletion]).unwrap();
            assert!(!apply_ops(&conn, &[parent]).unwrap().deferred);
            assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
            assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        }
    }

    #[test]
    fn raw_replay_resolves_pending_compacted_history_without_restoring_text() {
        for retention in [None, Some(7)] {
            let db = db::open(":memory:").unwrap();
            let conn = db.lock().unwrap();
            let receipt = json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","hour":10,"context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","words":5,"spoken_words":null,"duration_ms":1000});
            crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", receipt).unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Recovered')", []).unwrap();
            let op = SyncOp { table:"transcriptions".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"upsert".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:1, payload:Some(json!({"raw_text":"a","clean_text":"a","words":5,"duration_ms":1000,"api_used":"test","context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","created_at":"2020-01-01 10:00:00","activity_day":"2020-01-01","activity_hour":10})) };
            apply_ops_with_retention(&conn, &[op.clone()], retention).unwrap();
            apply_ops_with_retention(&conn, &[op], retention).unwrap();
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
            assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
            assert_eq!(conn.query_row("SELECT total_words FROM transcription_context_daily_stats WHERE context_uuid='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        }
    }

    #[test]
    fn expired_history_and_call_sync_with_missing_context() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',5,1000,'test',datetime('now','-40 days'))", []).unwrap();
            conn.execute("INSERT INTO pending_transcription_contexts VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',id,'model','provider','stt',datetime('now','-40 days') FROM transcriptions", []).unwrap();
        }
        let (ops, _, _) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        let conn = dest.lock().unwrap();
        assert!(!apply_ops_with_retention(&conn, &ops, Some(30)).unwrap().deferred);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcription_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM call_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn sync_parent_resolves_call_only_pending_context_before_pruning() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Synthetic')", []).unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,context_id,created_at,activity_day,activity_hour) SELECT 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','a','a',5,1000,'test',id,'2020-01-01 10:00:00','2020-01-01',10 FROM contexts WHERE name='Synthetic'", []).unwrap();
        }
        let call = json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","day":"2020-01-01","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        {
            let conn = dest.lock().unwrap();
            crate::commands::settings::import_sync_receipt(&conn, "call_receipts", call.clone()).unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Synthetic')", []).unwrap();
        }
        let (ops, _, _) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        let history: Vec<_> = ops.into_iter().filter(|op| op.table == "transcriptions").collect();
        apply_ops(&dest.lock().unwrap(), &history).unwrap();
        db::prune_transcriptions_older_than(&dest, 7).unwrap();
        let conn = dest.lock().unwrap();
        crate::commands::settings::import_sync_receipt(&conn, "call_receipts", call).unwrap();
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_context_daily_stats WHERE context_uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn compacted_held_calls_do_not_become_sync_deletions() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        let entry = sync_store::LogEntry { seq: 1, table_name: "held_api_calls".into(), row_uuid: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op: "upsert".into(), ts_ms: 1, origin: "peer".into(), origin_seq: 1 };
        assert!(resolve_entry(&conn, entry).unwrap().is_none());
    }

    #[test]
    fn forwarded_history_uses_the_strongest_local_hold() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO backup_held_history VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')", []).unwrap();
        conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',5,1000,'test','2020-01-01 10:00:00')", []).unwrap();
        let entry = sync_store::LogEntry { seq: 1, table_name: "held_hourly_transcriptions".into(), row_uuid: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op: "upsert".into(), ts_ms: 1, origin: "peer".into(), origin_seq: 1 };
        assert_eq!(resolve_entry(&conn, entry).unwrap().unwrap().table, "held_transcriptions");
    }

    #[test]
    fn delayed_context_preserves_hourly_hold_and_checks_provider_overlap() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Synthetic')", []).unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,context_id,created_at,activity_day,activity_hour) SELECT 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','a','a',5,1000,'test',id,'2020-01-01 10:00:00','2020-01-01',10 FROM contexts WHERE name='Synthetic'", []).unwrap();
            conn.execute("INSERT INTO backup_held_hourly_receipts VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at,activity_day) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',id,'model','provider','stt','2020-01-01 10:00:00','2020-01-01' FROM transcriptions", []).unwrap();
        }
        let (ops, _, _) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        let history: Vec<_> = ops.iter().filter(|op| matches!(op.table.as_str(), "held_hourly_transcriptions" | "api_calls")).cloned().collect();
        let contexts: Vec<_> = ops.iter().filter(|op| op.table == "contexts").cloned().collect();
        assert_eq!(history.len(), 2);
        let conn = dest.lock().unwrap();
        assert!(!apply_ops(&conn, &history).unwrap().deferred);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        sync_store::set_sync_applying(&conn, true).unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Synthetic')", []).unwrap();
        sync_store::set_sync_applying(&conn, false).unwrap();
        conn.execute("INSERT INTO provider_daily_stats SELECT '2020-01-01',id,'model','provider','stt',1,0,0,0 FROM contexts WHERE name='Synthetic'", []).unwrap();
        apply_ops(&conn, &contexts).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_held_hourly_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_held_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn pending_context_survives_retention_and_moves_compacted_usage() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        source.lock().unwrap().execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Synthetic')", []).unwrap();
        {
            let conn = dest.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at,activity_day,activity_hour) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','a','a',5,1000,'test','2020-01-01 10:00:00','2020-01-01',10)", []).unwrap();
            conn.execute("INSERT INTO pending_transcription_contexts VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at,activity_day) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',id,'model','provider','stt','2020-01-01 10:00:00','2020-01-01' FROM transcriptions", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at,activity_day) SELECT 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',id,'model','provider','stt','2020-01-01 10:00:00','2020-01-01' FROM transcriptions", []).unwrap();
        }
        db::compact_api_calls_older_than(&dest, 7).unwrap();
        db::prune_transcriptions_older_than(&dest, 7).unwrap();
        let (ops, _, _) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        let contexts: Vec<_> = ops.into_iter().filter(|op| op.table == "contexts").collect();
        let conn = dest.lock().unwrap();
        assert_eq!(conn.query_row("SELECT context_uuid FROM transcription_receipts", [], |r| r.get::<_, String>(0)).unwrap(), "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM call_receipts WHERE context_uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", json!({"uuid":"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee","day":"2020-01-01","hour":10,"context_uuid":null,"words":7,"spoken_words":null,"duration_ms":1000})).unwrap(), 0);
        assert_eq!(crate::commands::settings::import_sync_receipt(&conn, "call_receipts", json!({"uuid":"ffffffff-ffff-4fff-8fff-ffffffffffff","transcription_uuid":null,"day":"2020-01-01","context_uuid":null,"model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0})).unwrap(), 0);
        apply_ops(&conn, &contexts).unwrap();
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_context_daily_stats WHERE context_uuid='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_hourly_stats WHERE context_id=(SELECT id FROM contexts WHERE name='Synthetic')", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        assert_eq!(conn.query_row("SELECT calls FROM provider_daily_stats WHERE context_id=(SELECT id FROM contexts WHERE name='Synthetic')", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_hourly_stats WHERE context_id=0", [], |r| r.get::<_, i64>(0)).unwrap(), 7);
        assert_eq!(conn.query_row("SELECT calls FROM provider_daily_stats WHERE context_id=0", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM pending_transcription_contexts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn pending_retained_call_is_not_held_after_sibling_compaction() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        source.lock().unwrap().execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','Synthetic')", []).unwrap();
        {
            let conn = dest.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','a','a',5,1000,'test','2020-01-01 10:00:00')", []).unwrap();
            conn.execute("INSERT INTO pending_transcription_contexts VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at,activity_day) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',id,'model','provider','stt','2020-01-01 10:00:00','2020-01-01' FROM transcriptions", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at,activity_day) SELECT 'dddddddd-dddd-4ddd-8ddd-dddddddddddd',id,'model','provider','stt','2099-01-01 10:00:00','2020-01-01' FROM transcriptions", []).unwrap();
        }
        db::compact_api_calls_older_than(&dest, 7).unwrap();
        let (ops, _, _) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        let contexts: Vec<_> = ops.into_iter().filter(|op| op.table == "contexts").collect();
        let conn = dest.lock().unwrap();
        apply_ops(&conn, &contexts).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_held_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT calls FROM provider_daily_stats WHERE context_id=(SELECT id FROM contexts WHERE name='Synthetic')", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM api_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn held_history_is_excluded_when_best_wpm_is_recomputed() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO backup_held_history VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')", []).unwrap();
        conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',100,1000,'test','2020-01-01 10:00:00'),('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','b','b',2,1000,'test','2020-01-01 10:00:00'),('cccccccc-cccc-4ccc-8ccc-cccccccccccc','c','c',1,1000,'test','2020-01-01 10:00:00')", []).unwrap();
        conn.execute("DELETE FROM transcriptions WHERE uuid='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'", []).unwrap();
        let best: f64 = conn.query_row("SELECT best_wpm FROM transcription_daily_stats WHERE day='2020-01-01'", [], |r| r.get(0)).unwrap();
        assert_eq!(best, 60.0);
    }

    #[test]
    fn preupgrade_ambiguous_history_and_calls_sync_as_held() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        let conn = source.lock().unwrap();
        sync_store::ensure_self_identity(&conn, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "source").unwrap();
        conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at,activity_day,activity_hour) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','a','a',5,1000,'test','2020-01-01 01:00:00','2020-01-01',1)", []).unwrap();
        conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at,activity_day) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',id,'model','provider','stt','2020-01-01 01:00:00','2020-01-01' FROM transcriptions", []).unwrap();
        conn.execute("INSERT INTO backup_preupgrade_history VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')", []).unwrap();
        let (ops, _, _) = collect_ops(&conn, 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        assert!(ops.iter().any(|op| op.table == "held_transcriptions"));
        assert!(ops.iter().any(|op| op.table == "held_api_calls"));
        drop(conn);
        let conn = dest.lock().unwrap();
        apply_ops(&conn, &ops).unwrap();
        assert_eq!(conn.query_row("SELECT COALESCE(SUM(total_words),0) FROM transcription_daily_stats", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_held_calls", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
    }

    #[test]
    fn held_history_and_calls_sync_without_inflating_statistics() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        crate::sync::store::ensure_self_identity(&source.lock().unwrap(), "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "source").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO backup_held_history VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb')", []).unwrap();
            conn.execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', 'synthetic', 'synthetic', 5, 1000, 'test', '2020-01-01 12:00:00')", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid, transcription_id, model, provider, task, created_at) SELECT 'cccccccc-cccc-4ccc-8ccc-cccccccccccc', id, 'model', 'test', 'stt', '2020-01-01 12:00:00' FROM transcriptions", []).unwrap();
            conn.execute("INSERT INTO backup_held_calls VALUES ('cccccccc-cccc-4ccc-8ccc-cccccccccccc')", []).unwrap();
        }
        let (ops, _, done) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        assert!(done);
        assert!(ops.iter().any(|op| op.table == "held_transcriptions"));
        assert!(ops.iter().any(|op| op.table == "held_api_calls"));
        let conn = dest.lock().unwrap();
        apply_ops(&conn, &ops).unwrap();
        apply_ops(&conn, &ops).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM api_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COALESCE(SUM(total_words),0) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        drop(conn);
        let expired = db::open(":memory:").unwrap();
        let conn = expired.lock().unwrap();
        apply_ops_with_retention(&conn, &ops, Some(7)).unwrap();
        assert_eq!(conn.query_row("SELECT counted FROM call_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT COALESCE(SUM(calls),0) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn sync_compacts_incoming_history_outside_destination_retention() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        crate::sync::store::ensure_self_identity(&source.lock().unwrap(), "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "source").unwrap();
        source.lock().unwrap().execute("INSERT INTO transcriptions(uuid, raw_text, clean_text, words, duration_ms, api_used, created_at) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', 'synthetic', 'synthetic', 5, 1000, 'test', '2020-01-01 12:00:00')", []).unwrap();
        let (ops, _, _) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        let conn = dest.lock().unwrap();
        apply_ops_with_retention(&conn, &ops, Some(7)).unwrap();
        apply_ops_with_retention(&conn, &ops, Some(7)).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcription_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn retained_sync_keeps_legacy_provider_and_hourly_overlaps_uncounted() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        {
            let conn = source.lock().unwrap();
            conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used,created_at,activity_day,activity_hour) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',5,1000,'test','2020-01-01 10:00:00','2020-01-01',10)", []).unwrap();
            conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task,created_at,activity_day) SELECT 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',id,'model','provider','stt','2020-01-01 10:00:00','2020-01-01' FROM transcriptions", []).unwrap();
        }
        {
            let conn = dest.lock().unwrap();
            conn.execute("INSERT INTO transcription_hourly_stats VALUES ('2020-01-01',10,0,5)", []).unwrap();
            conn.execute("INSERT INTO provider_daily_stats VALUES ('2020-01-01',0,'model','provider','stt',1,0,0,0)", []).unwrap();
        }
        let (ops, _, _) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut SnapshotProgress::default()).unwrap();
        let conn = dest.lock().unwrap();
        apply_ops(&conn, &ops).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_held_hourly_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM backup_held_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(conn.query_row("SELECT total_words FROM transcription_daily_stats WHERE day='2020-01-01'", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
    }

    #[test]
    fn upgrading_a_peer_requests_one_new_snapshot() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        sync_store::upsert_peer(&conn, "peer", "peer", "fingerprint").unwrap();
        sync_store::set_peer_send_position(&conn, "peer", 42, false).unwrap();
        note_receipt_capability(&conn, "peer", false).unwrap();
        assert!(!sync_store::peer_send_position(&conn, "peer").unwrap().1);
        note_receipt_capability(&conn, "peer", true).unwrap();
        assert!(sync_store::peer_send_position(&conn, "peer").unwrap().1);
        sync_store::set_peer_send_position(&conn, "peer", 42, false).unwrap();
        note_receipt_capability(&conn, "peer", true).unwrap();
        assert!(!sync_store::peer_send_position(&conn, "peer").unwrap().1);
    }

    #[test]
    fn compacted_activity_syncs_in_snapshot_and_incremental_exchange() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        crate::sync::store::ensure_self_identity(&source.lock().unwrap(), "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "source").unwrap();
        crate::sync::store::ensure_self_identity(&dest.lock().unwrap(), "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "dest").unwrap();
        let entry = db::insert_transcription_returning(&source, "one two", "one two", 2, 1000, "test", None, None).unwrap();
        source.lock().unwrap().execute("UPDATE transcriptions SET created_at='2020-01-01 12:00:00' WHERE id=?1", [entry.id]).unwrap();
        db::insert_api_calls(&source, &[db::ApiCall { transcription_id:entry.id, model:"synthetic".into(), provider:"test".into(), task:"stt".into(), audio_ms:1000, input_chars:0, output_chars:1, created_at:"2020-01-01 12:00:00".into() }]).unwrap();
        db::prune_transcriptions_older_than(&source, 7).unwrap();
        let mut progress = SnapshotProgress::default();
        let (ops, _, done) = collect_ops(&source.lock().unwrap(), 0, true, 1000, &mut progress).unwrap();
        assert!(done);
        assert!(ops.iter().any(|op| op.table == "transcription_receipts"));
        assert!(ops.iter().any(|op| op.table == "call_receipts"));
        let conn = dest.lock().unwrap();
        apply_ops(&conn, &ops).unwrap();
        apply_ops(&conn, &ops).unwrap();
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_hourly_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        drop(conn);
        let (incremental, _, _) = collect_ops(&source.lock().unwrap(), 0, false, 1000, &mut SnapshotProgress::default()).unwrap();
        apply_ops(&dest.lock().unwrap(), &incremental).unwrap();
        assert_eq!(dest.lock().unwrap().query_row("SELECT SUM(total_words) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }

    #[test]
    fn sync_does_not_restore_a_pruned_transcription() {
        let db = db::open(":memory:").unwrap();
        let entry = db::insert_transcription_returning(&db, "synthetic", "synthetic", 1, 1000, "test", None, None).unwrap();
        let (uuid, payload) = {
            let conn = db.lock().unwrap();
            conn.execute("UPDATE transcriptions SET created_at='2020-01-01 12:00:00' WHERE id=?1", [entry.id]).unwrap();
            let uuid: String = conn.query_row("SELECT uuid FROM transcriptions WHERE id=?1", [entry.id], |r| r.get(0)).unwrap();
            let payload = row_payload(&conn, "transcriptions", &uuid).unwrap();
            (uuid, payload)
        };
        db::prune_transcriptions_older_than(&db, 7).unwrap();
        let conn = db.lock().unwrap();
        let op = SyncOp { table:"transcriptions".into(), row_uuid:uuid, op:"upsert".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:1, payload };
        assert!(matches!(apply_transcription_op(&conn, &op).unwrap(), Applied::Skipped));
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM transcriptions", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT SUM(total_words) FROM transcription_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn sync_keeps_usage_for_a_call_arriving_after_its_parent_was_pruned() {
        let source = db::open(":memory:").unwrap();
        let dest = db::open(":memory:").unwrap();
        let entry = db::insert_transcription_returning(&source, "synthetic", "synthetic", 1, 1000, "test", None, None).unwrap();
        source.lock().unwrap().execute("UPDATE transcriptions SET created_at='2020-01-01 12:00:00' WHERE id=?1", [entry.id]).unwrap();
        db::insert_api_calls(&source, &[db::ApiCall { transcription_id:entry.id, model:"synthetic".into(), provider:"test".into(), task:"stt".into(), audio_ms:1000, input_chars:0, output_chars:1, created_at:"2020-01-01 12:00:00".into() }]).unwrap();
        let (transcription, call) = {
            let conn = source.lock().unwrap();
            let id: String = conn.query_row("SELECT uuid FROM transcriptions WHERE id=?1", [entry.id], |r| r.get(0)).unwrap();
            let call_id: String = conn.query_row("SELECT uuid FROM api_calls LIMIT 1", [], |r| r.get(0)).unwrap();
            let make_op = |table: &str, uuid: String| SyncOp { table:table.into(), payload:row_payload(&conn, table, &uuid).unwrap(), row_uuid:uuid, op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1 };
            (make_op("transcriptions", id), make_op("api_calls", call_id))
        };
        apply_transcription_op(&dest.lock().unwrap(), &transcription).unwrap();
        db::prune_transcriptions_older_than(&dest, 7).unwrap();
        let conn = dest.lock().unwrap();
        assert!(matches!(apply_api_call_op(&conn, &call).unwrap(), Applied::Yes));
        assert!(matches!(apply_api_call_op(&conn, &call).unwrap(), Applied::Skipped));
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn sync_defers_calls_until_their_parent_arrives() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        let op = SyncOp { table:"api_calls".into(), row_uuid:"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(), op:"upsert".into(), ts_ms:1, origin:"peer".into(), origin_seq:1, payload:Some(json!({"transcription_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "model":"synthetic", "provider":"test", "task":"stt", "created_at":"2020-01-01 12:00:00"})) };
        assert!(matches!(apply_api_call_op(&conn, &op).unwrap(), Applied::Deferred));
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM call_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn raw_call_resolves_compacted_parent_context_before_counting() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        let parent = json!({"uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","hour":10,"context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","words":5,"spoken_words":null,"duration_ms":1000});
        let first = json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","day":"2020-01-01","context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","model":"synthetic","provider":"test","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", parent.clone()).unwrap();
        crate::commands::settings::import_sync_receipt(&conn, "call_receipts", first).unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Recovered')", []).unwrap();
        let op = SyncOp { table:"api_calls".into(), row_uuid:"dddddddd-dddd-4ddd-8ddd-dddddddddddd".into(), op:"upsert".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:1, payload:Some(json!({"transcription_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"synthetic","provider":"test","task":"stt","created_at":"2020-01-01 10:00:00","activity_day":"2020-01-01"})) };
        assert!(matches!(apply_api_call_op(&conn, &op).unwrap(), Applied::Yes));
        crate::commands::settings::import_sync_receipt(&conn, "transcription_receipts", parent).unwrap();
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM call_receipts WHERE counted=1", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }

    #[test]
    fn raw_call_does_not_duplicate_a_retained_uuid() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO transcriptions(uuid,raw_text,clean_text,words,duration_ms,api_used) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','a','a',1,1000,'test')", []).unwrap();
        conn.execute("INSERT INTO api_calls(uuid,transcription_id,model,provider,task) SELECT 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',id,'synthetic','test','stt' FROM transcriptions", []).unwrap();
        let op = SyncOp { table:"api_calls".into(), row_uuid:"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(), op:"upsert".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:1, payload:Some(json!({"transcription_uuid":null,"model":"synthetic","provider":"test","task":"stt","created_at":"2020-01-01 10:00:00"})) };
        assert!(matches!(apply_api_call_op(&conn, &op).unwrap(), Applied::Skipped));
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM call_receipts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM api_calls", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn retained_parent_rejects_context_conflicting_with_orphan_call() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute("INSERT INTO contexts(uuid,name) VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa','First'),('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb','Second')", []).unwrap();
        let call = json!({"uuid":"cccccccc-cccc-4ccc-8ccc-cccccccccccc","transcription_uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","day":"2020-01-01","context_uuid":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","model":"model","provider":"provider","task":"stt","audio_ms":0,"input_chars":0,"output_chars":0});
        crate::commands::settings::import_sync_receipt(&conn, "call_receipts", call).unwrap();
        let mut op = SyncOp { table:"transcriptions".into(), row_uuid:"dddddddd-dddd-4ddd-8ddd-dddddddddddd".into(), op:"upsert".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:1, payload:Some(json!({"raw_text":"a","clean_text":"a","words":5,"duration_ms":1000,"api_used":"test","context_uuid":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","created_at":"2020-01-01 10:00:00"})) };
        assert!(matches!(apply_transcription_op(&conn, &op).unwrap(), Applied::Skipped));
        let dependent = SyncOp { table:"api_calls".into(), row_uuid:"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".into(), op:"upsert".into(), ts_ms:i64::MAX, origin:"peer".into(), origin_seq:2, payload:Some(json!({"transcription_uuid":"dddddddd-dddd-4ddd-8ddd-dddddddddddd","model":"model","provider":"provider","task":"stt","created_at":"2020-01-01 10:00:00"})) };
        assert!(!apply_ops(&conn, &[dependent]).unwrap().deferred);
        op.payload.as_mut().unwrap()["context_uuid"] = json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        assert!(matches!(apply_transcription_op(&conn, &op).unwrap(), Applied::Yes));
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM sync_rejected_history", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT SUM(calls) FROM provider_daily_stats", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn peers_without_nullable_formatting_overrides_are_rejected() {
        let mut hello = Hello {
            device_uuid: "peer".into(),
            device_name: "Peer".into(),
            protocol: 2,
            app_version: "0.0.0".into(),
            activity_receipts: false,
        };
        assert!(check_hello(&hello, "peer").is_err());
        hello.protocol = PROTOCOL_VERSION;
        assert!(check_hello(&hello, "peer").is_ok());
    }

    #[test]
    fn sync_record_payloads_preserve_wire_bytes_and_round_trip() {
        let db = db::open(":memory:").unwrap();
        let conn = db.lock().unwrap();
        conn.execute_batch(
            "INSERT INTO contexts (id, uuid, name, icon, tone, cleanup_intensity, color,
                custom_instructions, contextual_formatting_disabled, paste_in_chunks, pinned_at,
                created_at, updated_at)
             VALUES (2, 'context', 'Work', 'W', 'casual', 'light', '#123456', 'Keep café',
                1, 1, '2026-01-02', '2026-01-01', '2026-01-02');
             INSERT INTO dictionary (id, uuid, term, mistake, auto_learned, correction_count,
                confidence_tier, last_seen_at, created_at)
             VALUES (1, 'dictionary', 'café', 'legacy mistake', 1, 7, 'high', '2026-01-02', '2026-01-01');
             INSERT INTO dictionary_contexts (context_id, dictionary_id) VALUES (2, 1);
             INSERT INTO dictionary_corrections (uuid, dictionary_id, context_id, mistake,
                auto_learned, correction_count, confidence_tier, last_seen_at, created_at)
             VALUES ('correction', 1, 2, 'cafe', 1, 3, 'medium', NULL, '2026-01-01');
             INSERT INTO snippets (id, uuid, trigger, expansion, instructions, use_count, created_at)
             VALUES (1, 'snippet', 'sig', 'Line one\nLine two', 'Keep lines', 4, '2026-01-01');
             INSERT INTO snippet_contexts (context_id, snippet_id) VALUES (2, 1);
             INSERT INTO context_targets (context_id, executable, platform, app_name, developer)
             VALUES (2, 'editor', 'linux', 'Editor', 'Example');
             INSERT INTO context_website_targets (context_id, domain) VALUES (2, 'example.com');
             INSERT INTO context_sub_apps (uuid, context_id, executable, platform, app_name,
                label, icon, title_pattern, match_mode, created_at)
             VALUES ('subapp', 2, 'editor', NULL, 'Editor', 'Work tabs', NULL, 'Work', 'contains', '2026-01-01');
             INSERT INTO transcriptions (id, uuid, context_id, raw_text, clean_text, words,
                spoken_words, duration_ms, api_used, app_name, created_at, activity_day, activity_hour)
             VALUES (1, 'transcription', 2, 'raw café', 'Clean café.', 2, 3, 1234, 'provider/model', NULL, '2026-01-01', '2026-01-01', 0);
             INSERT INTO api_calls (uuid, transcription_id, model, provider, task, audio_ms,
                input_chars, output_chars, created_at, activity_day)
             VALUES ('call', 1, 'model', 'provider', 'cleanup', 1234, 8, 11, '2026-01-01', '2026-01-01');",
        ).unwrap();
        let cases = [
            (
                "dictionary",
                "dictionary",
                json!({
                    "term": "café", "mistake": null, "auto_learned": true,
                    "correction_count": 7, "confidence_tier": "high", "last_seen_at": "2026-01-02",
                    "created_at": "2026-01-01"
                }),
            ),
            (
                "dictionary_corrections",
                "correction",
                json!({
                    "context_uuid": "context", "dictionary_uuid": "dictionary", "dictionary_term": "café",
                    "mistake": "cafe", "auto_learned": true, "correction_count": 3,
                    "confidence_tier": "medium", "last_seen_at": null, "created_at": "2026-01-01"
                }),
            ),
            (
                "snippets",
                "snippet",
                json!({
                    "trigger": "sig", "expansion": "Line one\nLine two", "instructions": "Keep lines",
                    "use_count": 4, "created_at": "2026-01-01"
                }),
            ),
            (
                "contexts",
                "context",
                json!({
                    "name": "Work", "is_everywhere": false, "icon": "W", "tone": "casual",
                    "cleanup_intensity": "light", "color": "#123456", "custom_instructions": "Keep café",
                    "contextual_formatting_disabled": true, "contextual_formatting_override": null, "paste_in_chunks": true,
                    "t3_skill_mentions_disabled": false,
                    "pinned_at": "2026-01-02", "created_at": "2026-01-01", "updated_at": "2026-01-02",
                    "targets": [{"executable": "editor", "platform": "linux", "app_name": "Editor", "developer": "Example"}],
                    "websites": ["example.com"], "dictionary_uuids": ["dictionary"],
                    "dictionary_entries": [{"uuid": "dictionary", "term": "café"}], "snippet_uuids": ["snippet"]
                }),
            ),
            (
                "context_sub_apps",
                "subapp",
                json!({
                    "executable": "editor", "platform": null, "app_name": "Editor", "label": "Work tabs",
                    "icon": null, "title_pattern": "Work", "match_mode": "contains",
                    "context_uuid": "context", "created_at": "2026-01-01"
                }),
            ),
            (
                "transcriptions",
                "transcription",
                json!({
                    "raw_text": "raw café", "clean_text": "Clean café.", "words": 2, "spoken_words": 3,
                    "duration_ms": 1234, "api_used": "provider/model", "app_name": null,
                    "context_uuid": "context", "created_at": "2026-01-01", "activity_day": "2026-01-01", "activity_hour": 0
                }),
            ),
            (
                "api_calls",
                "call",
                json!({
                    "transcription_uuid": "transcription", "model": "model", "provider": "provider",
                    "task": "cleanup", "audio_ms": 1234, "input_chars": 8, "output_chars": 11,
                    "created_at": "2026-01-01", "activity_day": "2026-01-01"
                }),
            ),
        ];
        for (table, uuid, expected) in cases {
            let actual = row_payload(&conn, table, uuid).unwrap().unwrap();
            assert_eq!(
                serde_json::to_vec(&actual).unwrap(),
                serde_json::to_vec(&expected).unwrap(),
                "{table}"
            );
            let op = SyncOp {
                table: table.into(),
                row_uuid: uuid.into(),
                op: "upsert".into(),
                ts_ms: 1,
                origin: "fixture".into(),
                origin_seq: 1,
                payload: Some(actual),
            };
            macro_rules! round_trip {
                ($ty:ty) => {{
                    let row: $ty = op_payload(&op, "missing", "invalid").unwrap();
                    serde_json::to_value(row).unwrap()
                }};
            }
            let decoded = match table {
                "dictionary" => round_trip!(DictionaryRow),
                "dictionary_corrections" => round_trip!(DictionaryCorrectionRow),
                "snippets" => round_trip!(SnippetRow),
                "contexts" => round_trip!(ContextAggregate),
                "context_sub_apps" => round_trip!(SubAppRow),
                "transcriptions" => round_trip!(TranscriptionRow),
                "api_calls" => round_trip!(ApiCallRow),
                _ => unreachable!(),
            };
            assert_eq!(
                serde_json::to_vec(&decoded).unwrap(),
                serde_json::to_vec(&expected).unwrap(),
                "{table} round trip"
            );
            assert!(
                row_payload(&conn, table, "absent").unwrap().is_none(),
                "{table} missing UUID"
            );
        }
        assert!(row_payload(&conn, "unknown", "dictionary")
            .unwrap()
            .is_none());
    }
}
