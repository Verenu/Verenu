//! Insights aggregation for the Insights page.
//!
//! Mirrors `src/lib/views/insights/types.ts` field for field — serde field
//! names are the frontend contract and must not drift. Timestamp filters use
//! UTC-naive range bounds so `created_at` remains seekable; day/hour bucketing
//! still goes through `date(created_at, 'localtime')` / `strftime('%H',
//! created_at, 'localtime')` because the stored value is UTC-naive (see
//! `query_stats` in transcriptions.rs).
//!
//! Privacy: this module only ever emits counts, model ids, and normalized
//! word stats. It never logs dictated text, clean_text, or top words.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Insights {
    /// `None` when the payload covers every context. When set, every
    /// per-dictation figure is scoped to it; the lifetime counters called out
    /// on `InsightsCleanup` are not.
    pub context_id: Option<i64>,
    pub range_days: i64,
    pub generated_at: String,
    pub totals: InsightsTotals,
    pub streak: InsightsStreak,
    pub daily: Vec<InsightsDay>,
    /// One compact rolling year. The frontend shows as many fixed-width weeks
    /// as fit without scrolling.
    pub streak_daily: Vec<InsightsDay>,
    pub history_started_on: Option<String>,
    pub hourly: Vec<i64>,
    pub providers: Vec<InsightsProviderUsage>,
    pub cleanup: InsightsCleanup,
    pub words: InsightsWords,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InsightsTotals {
    pub total_words: i64,
    pub total_transcriptions: i64,
    pub total_speaking_ms: i64,
    pub avg_words_per_transcription: i64,
    pub avg_wpm: f64,
    pub best_wpm: i64,
    pub words_in_range: i64,
    pub words_prev_range: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InsightsStreak {
    pub current_days: i64,
    pub longest_days: i64,
    pub longest_started_on: Option<String>,
    pub longest_ended_on: Option<String>,
    pub longest_words: i64,
    pub active_days: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InsightsDay {
    pub day: String,
    pub words: i64,
    pub transcriptions: i64,
    pub speaking_ms: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InsightsProviderUsage {
    pub model: String,
    pub provider: String,
    pub task: String,
    pub calls: i64,
    pub audio_ms: i64,
    pub input_chars: i64,
    pub output_chars: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InsightsCleanup {
    pub raw_words: i64,
    pub clean_words: i64,
    pub edits_applied: i64,
    pub dictionary_fixes: i64,
    pub auto_learned_terms: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InsightsWords {
    pub top: Vec<InsightsWordCount>,
    pub unique_words: i64,
    pub longest_word: Option<String>,
    pub avg_word_length: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InsightsWordCount {
    pub word: String,
    pub count: i64,
}

/// One provider call recorded at pipeline finalize time, aggregated into
/// `Insights.providers`. `audio_ms` is set for transcription tasks only;
/// `input_chars`/`output_chars` for cleanup tasks only.
#[derive(Debug, Clone)]
pub struct ApiCall {
    pub transcription_id: i64,
    pub model: String,
    pub provider: String,
    pub task: String,
    pub audio_ms: i64,
    pub input_chars: i64,
    pub output_chars: i64,
    pub created_at: String,
}

const TOP_WORDS_LIMIT: usize = 12;
const MIN_WORD_CHARS: usize = 3;
#[derive(Debug, Clone)]
struct DateRange {
    start_day: NaiveDate,
    end_day: NaiveDate,
    start: String,
    end: String,
}

#[derive(Debug)]
struct CleanupLifetime {
    dictionary_fixes: i64,
    auto_learned_terms: i64,
}

// Filler/function words users don't need to see in their distinctive
// vocabulary. Lowercase; `top`/`unique_words`/`avg_word_length` filter on it.
const STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be",
    "because", "been", "before", "being", "but", "by", "could", "did", "do", "does", "for", "from",
    "had", "has", "have", "he", "her", "hers", "him", "his", "how", "i", "if", "in", "into", "is",
    "it", "its", "me", "more", "most", "my", "no", "not", "of", "off", "on", "or", "our", "ours",
    "out", "over", "own", "said", "she", "so", "some", "than", "that", "the", "their", "them",
    "then", "there", "these", "they", "this", "those", "through", "to", "too", "under", "up", "us",
    "was", "we", "were", "what", "when", "where", "which", "while", "who", "whom", "why", "will",
    "with", "would", "you", "your", "yours",
];

fn is_stopword(word: &str) -> bool {
    STOPWORDS.binary_search(&word).is_ok()
}

/// Aggregated insights for `days` (`0` = all time). A brand-new install with
/// no transcriptions returns a fully-populated zero payload, never an error.
/// Aggregates for `days` (`0` = all time), optionally narrowed to one context.
///
/// Every per-dictation figure honours `context_id`. Dictionary and auto-learn
/// counters have no context dimension in the schema and stay global; the
/// `edits_applied` headline is the raw-to-final word diff in the selected range.
/// either way — the UI labels them as such.
pub fn query_insights(db: &Db, days: i64, context_id: Option<i64>) -> Result<Insights> {
    let (
        totals,
        daily,
        streak_daily,
        lifetime_streak_daily,
        history_started_on,
        hourly,
        providers,
        words,
        raw_words,
        clean_words,
        changed_words,
        cleanup_lifetime,
    ) = {
        let conn = lock_conn(db)?;
        let range = range_bounds(&conn, days, context_id)?;
        let previous = previous_range(days);
        let totals = query_totals(&conn, &range, previous.as_ref(), context_id)?;
        let lifetime_range = range_bounds(&conn, 0, context_id)?;
        // The selected window is always a suffix of the lifetime window. One
        // grouped query therefore supplies both series, including the idle
        // prefix needed when a new install has less than `days` of history.
        let aggregate_start = range.start_day.min(lifetime_range.start_day);
        let aggregate_range = make_date_range(aggregate_start, range.end_day);
        let aggregate_daily = query_daily(&conn, &aggregate_range, context_id)?;
        let daily = slice_daily(&aggregate_daily, aggregate_start, &range);
        let lifetime_streak_daily = slice_daily(&aggregate_daily, aggregate_start, &lifetime_range);
        // The lifetime daily series already contains every bucket needed for
        // the streak calculation. Derive the compact heatmap from it instead
        // of issuing a second overlapping 365-day aggregation query.
        let streak_daily = compact_streak_daily(&lifetime_streak_daily);
        // Scoped too, so the heatmap can still tell "before this context
        // existed" apart from "a day you didn't use it".
        let context_uuid: Option<String> = context_id
            .map(|id| {
                conn.query_row(
                    "SELECT uuid FROM contexts WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()
            })
            .transpose()?
            .flatten();
        let history_started_on: Option<String> = if let Some(uuid) = context_uuid.as_deref() {
            conn.query_row(
                "SELECT MIN(day) FROM transcription_context_daily_stats WHERE context_uuid = ?1",
                params![uuid],
                |r| r.get(0),
            )?
        } else {
            conn.query_row("SELECT MIN(day) FROM transcription_daily_stats", [], |r| {
                r.get(0)
            })?
        };
        let hourly = query_hourly(&conn, &range, context_id)?;
        let providers = query_providers(&conn, &range, context_id)?;
        let (words, raw_words, clean_words, changed_words) =
            query_text_metrics(&conn, &range, context_id)?;
        let cleanup_lifetime = query_cleanup_lifetime(&conn, context_id)?;

        (
            totals,
            daily,
            streak_daily,
            lifetime_streak_daily,
            history_started_on,
            hourly,
            providers,
            words,
            raw_words,
            clean_words,
            changed_words,
            cleanup_lifetime,
        )
    };

    let cleanup = InsightsCleanup {
        raw_words,
        clean_words,
        edits_applied: changed_words,
        dictionary_fixes: cleanup_lifetime.dictionary_fixes,
        auto_learned_terms: cleanup_lifetime.auto_learned_terms,
    };
    let streak = compute_streak(&lifetime_streak_daily);

    Ok(Insights {
        context_id,
        range_days: days,
        generated_at: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        totals,
        streak,
        daily,
        streak_daily,
        history_started_on,
        hourly,
        providers,
        cleanup,
        words,
    })
}

/// Inserts the per-call API usage rows for one transcription.
pub fn insert_api_calls(db: &Db, calls: &[ApiCall]) -> Result<()> {
    if calls.is_empty() {
        return Ok(());
    }
    let mut conn = lock_conn(db)?;
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO api_calls \
             (transcription_id, model, provider, task, audio_ms, input_chars, output_chars, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for call in calls {
            stmt.execute(params![
                call.transcription_id,
                call.model,
                call.provider,
                call.task,
                call.audio_ms,
                call.input_chars,
                call.output_chars,
                call.created_at,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Builds `ApiCall` rows for one finalized transcription by parsing the
/// `api_used` string the pipeline assembled (formats: `provider/model`,
/// `provider/model/transcription`, `primary=..;secondary=..`, with an
/// optional `;cleanup=..` suffix). Unparseable strings yield no rows, never
/// an error — cost accounting is best-effort and must not fail a dictation.
///
/// `audio_ms` is attributed to every transcription model (each one processed
/// the same clip); cleanup rows carry `input_chars`/`output_chars` derived
/// from the raw and cleaned text.
pub(crate) fn build_api_calls(
    transcription_id: i64,
    created_at: &str,
    api_used: &str,
    duration_ms: i64,
    raw: &str,
    clean: &str,
) -> Vec<ApiCall> {
    let parts = parse_api_usage(api_used);
    let mut calls =
        Vec::with_capacity(parts.transcription_models.len() + parts.cleanup_models.len());
    for (provider, model) in parts.transcription_models {
        calls.push(ApiCall {
            transcription_id,
            model,
            provider,
            task: "transcription".to_string(),
            audio_ms: duration_ms,
            input_chars: 0,
            output_chars: 0,
            created_at: created_at.to_string(),
        });
    }
    if !parts.cleanup_models.is_empty() {
        let input_chars = raw.chars().count() as i64;
        let output_chars = clean.chars().count() as i64;
        for (provider, model) in parts.cleanup_models {
            calls.push(ApiCall {
                transcription_id,
                model,
                provider,
                task: "cleanup".to_string(),
                audio_ms: 0,
                input_chars,
                output_chars,
                created_at: created_at.to_string(),
            });
        }
    }
    calls
}

/// Models parsed out of an `api_used` string, split by task. Each entry is
/// `(provider, model)`.
struct ApiUsageParts {
    transcription_models: Vec<(String, String)>,
    cleanup_models: Vec<(String, String)>,
}

/// Splits an `api_used` string into transcription and cleanup model pairs.
fn parse_api_usage(api_used: &str) -> ApiUsageParts {
    let (transcription_part, cleanup_part) = match api_used.split_once(";cleanup=") {
        Some((before, after)) => (before, Some(after)),
        None => (api_used, None),
    };

    let mut transcription = Vec::new();
    if let Some(primary) = transcription_part.strip_prefix("primary=") {
        for segment in primary.split(';') {
            let segment = segment.strip_prefix("secondary=").unwrap_or(segment);
            if let Some((provider, model)) = segment.split_once('/') {
                // Uniform with the single-provider fallback below: drop a
                // trailing /transcription so the model key is identical
                // regardless of which format produced the row.
                let model = model.strip_suffix("/transcription").unwrap_or(model);
                if !provider.is_empty() && !model.is_empty() {
                    transcription.push((provider.to_string(), model.to_string()));
                }
            }
        }
    } else if let Some((provider, rest)) = transcription_part.split_once('/') {
        let model = rest.strip_suffix("/transcription").unwrap_or(rest);
        if !provider.is_empty() && !model.is_empty() {
            transcription.push((provider.to_string(), model.to_string()));
        }
    }

    let mut cleanup = Vec::new();
    if let Some(segment) = cleanup_part {
        if let Some((provider, model)) = segment.split_once('/') {
            if !provider.is_empty() && !model.is_empty() {
                cleanup.push((provider.to_string(), model.to_string()));
            }
        }
    }

    ApiUsageParts {
        transcription_models: transcription,
        cleanup_models: cleanup,
    }
}

/// Local calendar-day bounds plus UTC-naive timestamp bounds. `created_at` is
/// stored as a UTC-naive value, so the timestamp predicates below can seek on
/// the ordinary `created_at` index without applying a function to the column.
fn range_bounds(conn: &Connection, days: i64, context_id: Option<i64>) -> Result<DateRange> {
    let today = Local::now().date_naive();
    let start_day = if days > 0 {
        today - Duration::days((days - 1).max(0))
    } else {
        if context_id.is_none() {
            conn.query_row("SELECT MIN(day) FROM transcription_daily_stats", [], |r| {
                r.get::<_, Option<String>>(0)
            })?
            .map(|day| NaiveDate::parse_from_str(&day, "%Y-%m-%d"))
            .transpose()?
            .unwrap_or(today)
        } else {
            let context_uuid: Option<String> = conn
                .query_row(
                    "SELECT uuid FROM contexts WHERE id = ?1",
                    params![context_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(context_uuid) = context_uuid {
                conn.query_row(
                    "SELECT MIN(day) FROM transcription_context_daily_stats WHERE context_uuid = ?1",
                    params![context_uuid],
                    |r| r.get::<_, Option<String>>(0),
                )?
                .map(|day| NaiveDate::parse_from_str(&day, "%Y-%m-%d"))
                .transpose()?
                .unwrap_or(today)
            } else {
                today
            }
        }
    };
    Ok(make_date_range(start_day, today))
}

/// One compact rolling year ending today. The frontend clips this to however
/// many fixed-width weeks fit in the current window.
fn rolling_year_bounds() -> DateRange {
    let end_day = Local::now().date_naive();
    make_date_range(end_day - Duration::days(364), end_day)
}

fn previous_range(days: i64) -> Option<DateRange> {
    if days <= 0 {
        return None;
    }
    let today = Local::now().date_naive();
    let end_day = today - Duration::days(days);
    Some(make_date_range(end_day - Duration::days(days - 1), end_day))
}

fn make_date_range(start_day: NaiveDate, end_day: NaiveDate) -> DateRange {
    let start = local_midnight_utc_naive(start_day);
    let end = local_midnight_utc_naive(end_day + Duration::days(1));
    DateRange {
        start_day,
        end_day,
        start: start.format("%Y-%m-%d %H:%M:%S").to_string(),
        end: end.format("%Y-%m-%d %H:%M:%S").to_string(),
    }
}

fn local_midnight_utc_naive(day: NaiveDate) -> NaiveDateTime {
    let midnight = day.and_hms_opt(0, 0, 0).expect("valid local midnight");
    // Midnight transitions are unusual but legal. For a nonexistent local
    // midnight, use the first representable wall-clock hour on that date.
    (0..6)
        .find_map(|hour| {
            let wall = day.and_hms_opt(hour, 0, 0)?;
            Local
                .from_local_datetime(&wall)
                .earliest()
                .map(|instant| instant.naive_utc())
        })
        .unwrap_or_else(|| {
            Local
                .from_local_datetime(&midnight)
                .latest()
                .expect("local timezone must resolve a day boundary")
                .naive_utc()
        })
}

fn query_totals(
    conn: &Connection,
    range: &DateRange,
    previous: Option<&DateRange>,
    context_id: Option<i64>,
) -> Result<InsightsTotals> {
    if context_id.is_none() {
        // The daily table is the source of truth for unscoped totals once raw
        // transcript text has aged out of retention.
        let previous_start = previous.map(|r| r.start_day.format("%Y-%m-%d").to_string());
        let previous_end = previous.map(|r| r.end_day.format("%Y-%m-%d").to_string());
        let (total_transcriptions, words_in_range, total_speaking_ms, wpm_sum, wpm_count, best_wpm, words_prev_range): (i64, i64, i64, f64, i64, f64, i64) = conn.query_row(
            "SELECT
               COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN total_transcriptions ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN total_words ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN speaking_ms ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN wpm_sum ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN wpm_count ELSE 0 END), 0),
               COALESCE(MAX(CASE WHEN day >= ?1 AND day <= ?2 THEN best_wpm ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN ?3 IS NOT NULL AND day >= ?3 AND day <= ?4 THEN total_words ELSE 0 END), 0)
             FROM transcription_daily_stats",
            params![
                range.start_day.format("%Y-%m-%d").to_string(),
                range.end_day.format("%Y-%m-%d").to_string(),
                previous_start,
                previous_end
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )?;
        let total_words: i64 = conn.query_row(
            "SELECT COALESCE((SELECT total_words FROM lifetime_stats WHERE id = 1), 0)
                  + COALESCE((SELECT SUM(total_words) FROM sync_remote_stats), 0)",
            [],
            |r| r.get(0),
        )?;
        return Ok(InsightsTotals {
            total_words,
            total_transcriptions,
            total_speaking_ms,
            avg_words_per_transcription: if total_transcriptions > 0 {
                (words_in_range as f64 / total_transcriptions as f64).round() as i64
            } else {
                0
            },
            avg_wpm: if wpm_count > 0 {
                wpm_sum / wpm_count as f64
            } else {
                0.0
            },
            best_wpm: best_wpm as i64,
            words_in_range,
            words_prev_range,
        });
    }
    let Some(context_uuid): Option<String> = conn
        .query_row(
            "SELECT uuid FROM contexts WHERE id = ?1",
            params![context_id],
            |r| r.get(0),
        )
        .optional()?
    else {
        return Ok(InsightsTotals {
            total_words: 0,
            avg_words_per_transcription: 0,
            total_transcriptions: 0,
            total_speaking_ms: 0,
            avg_wpm: 0.0,
            best_wpm: 0,
            words_in_range: 0,
            words_prev_range: 0,
        });
    };
    let previous_start = previous.map(|r| r.start_day.format("%Y-%m-%d").to_string());
    let previous_end = previous.map(|r| r.end_day.format("%Y-%m-%d").to_string());
    let (total_transcriptions, words_in_range, total_speaking_ms, wpm_sum, wpm_count, best_wpm, words_prev_range): (i64, i64, i64, f64, i64, f64, i64) = conn.query_row(
        "SELECT
           COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN total_transcriptions ELSE 0 END), 0),
           COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN total_words ELSE 0 END), 0),
           COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN speaking_ms ELSE 0 END), 0),
           COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN wpm_sum ELSE 0 END), 0),
           COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN wpm_count ELSE 0 END), 0),
           COALESCE(MAX(CASE WHEN day >= ?1 AND day <= ?2 THEN best_wpm ELSE 0 END), 0),
           COALESCE(SUM(CASE WHEN ?3 IS NOT NULL AND day >= ?3 AND day <= ?4 THEN total_words ELSE 0 END), 0)
         FROM transcription_context_daily_stats
        WHERE context_uuid = ?5",
        params![range.start_day.to_string(), range.end_day.to_string(), previous_start, previous_end, context_uuid],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
    )?;
    let total_words: i64 = conn.query_row(
        "SELECT COALESCE(SUM(total_words), 0) FROM transcription_context_daily_stats WHERE context_uuid = ?1",
        params![context_uuid],
        |r| r.get(0),
    )?;
    return Ok(InsightsTotals {
        total_words,
        total_transcriptions,
        total_speaking_ms,
        avg_words_per_transcription: if total_transcriptions > 0 {
            (words_in_range as f64 / total_transcriptions as f64).round() as i64
        } else {
            0
        },
        avg_wpm: if wpm_count > 0 {
            wpm_sum / wpm_count as f64
        } else {
            0.0
        },
        best_wpm: best_wpm as i64,
        words_in_range,
        words_prev_range,
    });
    /* // Legacy raw-row totals implementation retained only in history for reference.
    // Identical definition to query_stats (average of each clip's own wpm)
    // so the Insights "All time" number matches the Home page exactly;
    // scoped to the range here. Not total_words/total_duration — that
    // aggregates differently and makes the two pages disagree.
    let total_words: i64 = match context_id {
        None => conn.query_row(
            "SELECT COALESCE((SELECT total_words FROM lifetime_stats WHERE id = 1), 0)
                  + COALESCE((SELECT SUM(total_words) FROM sync_remote_stats), 0)",
            [],
            |r| r.get(0),
        )?,
        // lifetime_stats is a global counter with no context dimension, so a
        // scoped run sums the context's own history rather than reporting a
        // number the filter plainly does not apply to. It can read lower than
        // the unscoped lifetime figure, which never shrinks with retention
        // pruning — that difference is real, not a bug.
        Some(id) => conn.query_row(
            "SELECT COALESCE(SUM(words), 0) FROM transcriptions WHERE context_id = ?1",
            params![id],
            |r| r.get(0),
        )?,
    };

    let avg_words_per_transcription = if total_transcriptions > 0 {
        (words_in_range as f64 / total_transcriptions as f64).round() as i64
    } else {
        0
    };

    Ok(InsightsTotals {
        total_words,
        total_transcriptions,
        total_speaking_ms,
        avg_words_per_transcription,
        avg_wpm,
        best_wpm: best_wpm.unwrap_or(0.0) as i64,
        words_in_range,
        words_prev_range,
    }) */
}

/// One row per calendar day in the range, ascending, zero-filled for idle days.
fn query_daily(
    conn: &Connection,
    range: &DateRange,
    context_id: Option<i64>,
) -> Result<Vec<InsightsDay>> {
    let mut per_day: HashMap<String, (i64, i64, i64)> = HashMap::new();
    {
        // Retention may remove every raw row for an old day. The unscoped
        // Insights view must read the durable daily summary in that case.
        let sql = if context_id.is_none() {
            "SELECT day, total_words, total_transcriptions, speaking_ms
               FROM transcription_daily_stats
              WHERE day >= ?1 AND day <= ?2 AND ?3 IS NULL"
        } else {
            "SELECT day, total_words, total_transcriptions, speaking_ms
               FROM transcription_context_daily_stats
              WHERE day >= date(?1, 'localtime') AND day <= date(?2, 'localtime')
                AND context_uuid = (SELECT uuid FROM contexts WHERE id = ?3)"
        };
        let mut stmt = conn.prepare(sql)?;
        let start_bound = if context_id.is_none() {
            range.start_day.format("%Y-%m-%d").to_string()
        } else {
            range.start.clone()
        };
        let end_bound = if context_id.is_none() {
            range.end_day.format("%Y-%m-%d").to_string()
        } else {
            range.end.clone()
        };
        let rows = stmt.query_map(params![start_bound, end_bound, context_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ),
            ))
        })?;
        for row in rows {
            let (day, sums) = row?;
            per_day.insert(day, sums);
        }
    }

    let start = range.start_day;
    let end = range.end_day;
    if start > end {
        // A future-dated transcription (clock drift) can make the range
        // bounds inverted; never spin the zero-fill loop to NaiveDate::MAX.
        return Ok(Vec::new());
    }
    let span_days = (end - start).num_days();
    let mut daily = Vec::with_capacity(span_days as usize + 1);
    let mut current = start;
    loop {
        let day = current.format("%Y-%m-%d").to_string();
        let (words, transcriptions, speaking_ms) = per_day.get(&day).copied().unwrap_or((0, 0, 0));
        daily.push(InsightsDay {
            day,
            words,
            transcriptions,
            speaking_ms,
        });
        if current == end {
            break;
        }
        current = current.succ_opt().ok_or_else(|| {
            anyhow::anyhow!("insights: date overflow while zero-filling daily series")
        })?;
    }
    debug_assert_eq!(daily.len() as i64, span_days + 1);
    Ok(daily)
}

fn slice_daily(full: &[InsightsDay], full_start: NaiveDate, range: &DateRange) -> Vec<InsightsDay> {
    let start = (range.start_day - full_start).num_days().max(0) as usize;
    let len = (range.end_day - range.start_day).num_days().max(0) as usize + 1;
    full.iter().skip(start).take(len).cloned().collect()
}

fn compact_streak_daily(lifetime_daily: &[InsightsDay]) -> Vec<InsightsDay> {
    let range = rolling_year_bounds();
    let by_day: HashMap<&str, (i64, i64, i64)> = lifetime_daily
        .iter()
        .map(|day| {
            (
                day.day.as_str(),
                (day.words, day.transcriptions, day.speaking_ms),
            )
        })
        .collect();
    let mut daily = Vec::with_capacity(365);
    let mut current = range.start_day;
    loop {
        let day = current.format("%Y-%m-%d").to_string();
        let (words, transcriptions, speaking_ms) =
            by_day.get(day.as_str()).copied().unwrap_or((0, 0, 0));
        daily.push(InsightsDay {
            day,
            words,
            transcriptions,
            speaking_ms,
        });
        if current == range.end_day {
            break;
        }
        current = current
            .succ_opt()
            .expect("rolling streak range must fit in a calendar year");
    }
    daily
}

/// Words per hour-of-day, local time; exactly 24 entries, zeros included.
fn query_hourly(conn: &Connection, range: &DateRange, context_id: Option<i64>) -> Result<Vec<i64>> {
    let mut hourly = vec![0i64; 24];
    let mut stmt = conn.prepare(
        "SELECT CAST(strftime('%H', created_at, 'localtime') AS INTEGER), SUM(words)
         FROM transcriptions
         WHERE created_at >= ?1 AND created_at < ?2
           AND (?3 IS NULL OR context_id = ?3)
         GROUP BY 1",
    )?;
    let rows = stmt.query_map(params![range.start, range.end, context_id], |r| {
        Ok((r.get::<_, usize>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (hour, words) = row?;
        if let Some(slot) = hourly.get_mut(hour) {
            *slot = words;
        }
    }
    let mut compacted = conn.prepare(
        "SELECT hour, SUM(total_words)
           FROM transcription_hourly_stats
          WHERE day >= ?1 AND day <= ?2
            AND (?3 IS NULL OR context_id = COALESCE(?3, 0))
          GROUP BY hour",
    )?;
    let rows = compacted.query_map(
        params![
            range.start_day.to_string(),
            range.end_day.to_string(),
            context_id
        ],
        |r| Ok((r.get::<_, usize>(0)?, r.get::<_, i64>(1)?)),
    )?;
    for row in rows {
        let (hour, words) = row?;
        if let Some(slot) = hourly.get_mut(hour) {
            *slot += words;
        }
    }
    Ok(hourly)
}

/// Per-model/per-task usage from the `api_calls` table, in range. Rows
/// predating the table simply contribute nothing.
fn query_providers(
    conn: &Connection,
    range: &DateRange,
    context_id: Option<i64>,
) -> Result<Vec<InsightsProviderUsage>> {
    // api_calls has no context of its own; it inherits the one recorded on the
    // dictation that produced the call. LEFT JOIN so an unscoped run still
    // counts calls whose transcription has since been pruned.
    let mut stmt = conn.prepare(
        "SELECT a.model, a.provider, a.task, COUNT(*),
                COALESCE(SUM(a.audio_ms), 0),
                COALESCE(SUM(a.input_chars), 0),
                COALESCE(SUM(a.output_chars), 0)
         FROM api_calls a
         LEFT JOIN transcriptions t ON t.id = a.transcription_id
         WHERE a.created_at >= ?1 AND a.created_at < ?2
           AND (?3 IS NULL OR t.context_id = ?3)
         GROUP BY a.model, a.provider, a.task
         ORDER BY COUNT(*) DESC, a.model ASC",
    )?;
    let rows = stmt.query_map(params![range.start, range.end, context_id], |r| {
        Ok(InsightsProviderUsage {
            model: r.get(0)?,
            provider: r.get(1)?,
            task: r.get(2)?,
            calls: r.get(3)?,
            audio_ms: r.get(4)?,
            input_chars: r.get(5)?,
            output_chars: r.get(6)?,
        })
    })?;
    let mut by_key: HashMap<(String, String, String), InsightsProviderUsage> = HashMap::new();
    for row in rows {
        let value = row?;
        let key = (
            value.model.clone(),
            value.provider.clone(),
            value.task.clone(),
        );
        let entry = by_key.entry(key).or_insert_with(|| InsightsProviderUsage {
            model: value.model.clone(),
            provider: value.provider.clone(),
            task: value.task.clone(),
            calls: 0,
            audio_ms: 0,
            input_chars: 0,
            output_chars: 0,
        });
        entry.calls += value.calls;
        entry.audio_ms += value.audio_ms;
        entry.input_chars += value.input_chars;
        entry.output_chars += value.output_chars;
    }
    let mut compacted = conn.prepare(
        "SELECT model, provider, task, SUM(calls), SUM(audio_ms), SUM(input_chars), SUM(output_chars)
           FROM provider_daily_stats
          WHERE day >= ?1 AND day <= ?2
            AND (?3 IS NULL OR context_id = COALESCE(?3, 0))
          GROUP BY model, provider, task
          ORDER BY SUM(calls) DESC, model ASC",
    )?;
    let rows = compacted.query_map(
        params![
            range.start_day.to_string(),
            range.end_day.to_string(),
            context_id
        ],
        |r| {
            Ok(InsightsProviderUsage {
                model: r.get(0)?,
                provider: r.get(1)?,
                task: r.get(2)?,
                calls: r.get(3)?,
                audio_ms: r.get(4)?,
                input_chars: r.get(5)?,
                output_chars: r.get(6)?,
            })
        },
    )?;
    for row in rows {
        let value = row?;
        let key = (
            value.model.clone(),
            value.provider.clone(),
            value.task.clone(),
        );
        let entry = by_key.entry(key).or_insert_with(|| InsightsProviderUsage {
            model: value.model.clone(),
            provider: value.provider.clone(),
            task: value.task.clone(),
            calls: 0,
            audio_ms: 0,
            input_chars: 0,
            output_chars: 0,
        });
        entry.calls += value.calls;
        entry.audio_ms += value.audio_ms;
        entry.input_chars += value.input_chars;
        entry.output_chars += value.output_chars;
    }
    let mut usage: Vec<_> = by_key.into_values().collect();
    usage.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.model.cmp(&b.model)));
    Ok(usage)
}

fn query_cleanup_lifetime(conn: &Connection, context_id: Option<i64>) -> Result<CleanupLifetime> {
    let dictionary_fixes: i64 = match context_id {
        None => conn.query_row(
            "SELECT COALESCE((SELECT dictionary_fixes FROM lifetime_stats WHERE id = 1), 0)
                  + COALESCE((SELECT SUM(dictionary_fixes) FROM sync_remote_stats), 0)",
            [],
            |r| r.get(0),
        )?,
        Some(_) => conn.query_row(
            "SELECT COALESCE((SELECT dictionary_fixes FROM lifetime_stats WHERE id = 1), 0)",
            [],
            |r| r.get(0),
        )?,
    };
    let auto_learned_terms: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dictionary WHERE auto_learned = 1",
        [],
        |r| r.get(0),
    )?;
    Ok(CleanupLifetime {
        dictionary_fixes,
        auto_learned_terms,
    })
}

/// Count words and vocabulary in one cursor pass. Retain distinct-word counts,
/// not all transcript text; preserve the product's punctuation and accent rules.
fn query_text_metrics(
    conn: &Connection,
    range: &DateRange,
    context_id: Option<i64>,
) -> Result<(InsightsWords, i64, i64, i64)> {
    // This intentionally uses the same normalization as existing Insights
    // results. SQLite's unicode tokenizer splits `re-enter` and folds `café`,
    // so an FTS vocabulary index would make statistics depend on availability.
    let mut stmt = conn.prepare(
        "SELECT raw_text, clean_text FROM transcriptions
         WHERE created_at >= ?1 AND created_at < ?2
           AND (?3 IS NULL OR context_id = ?3)",
    )?;
    let mut rows = stmt.query(params![range.start, range.end, context_id])?;
    let mut counts: HashMap<String, i64> = HashMap::new();
    let mut longest: Option<String> = None;
    let mut length_sum = 0u64;
    let mut length_count = 0u64;
    let mut raw_words = 0i64;
    let mut clean_words = 0i64;
    let mut changed_words = 0i64;

    while let Some(row) = rows.next()? {
        let raw_text: String = row.get(0)?;
        let clean_text: String = row.get(1)?;
        raw_words += raw_text.split_whitespace().count() as i64;
        changed_words += count_changed_words(&raw_text, &clean_text);
        for token in clean_text.split_whitespace() {
            clean_words += 1;
            let normalized = normalize_word(token);
            if normalized.is_empty() {
                continue;
            }
            let char_len = normalized.chars().count();
            if char_len < MIN_WORD_CHARS || is_stopword(&normalized) {
                continue;
            }
            if char_len > longest.as_ref().map_or(0, |w| w.chars().count()) {
                longest = Some(normalized.clone());
            }
            *counts.entry(normalized).or_insert(0) += 1;
            length_sum += char_len as u64;
            length_count += 1;
        }
    }

    let unique_words = counts.len() as i64;
    let mut top: Vec<_> = counts
        .into_iter()
        .map(|(word, count)| InsightsWordCount { word, count })
        .collect();
    top.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.word.cmp(&b.word)));
    top.truncate(TOP_WORDS_LIMIT);
    Ok((
        InsightsWords {
            top,
            unique_words,
            longest_word: longest,
            avg_word_length: if length_count > 0 {
                length_sum as f64 / length_count as f64
            } else {
                0.0
            },
        },
        raw_words,
        clean_words,
        changed_words,
    ))
}

/// Returns the number of word insertions, deletions, and substitutions needed
/// to turn the best raw transcript into the final injected text. Comparison is
/// case- and punctuation-insensitive because this metric is about changed
/// words, not formatting changes.
fn count_changed_words(raw: &str, clean: &str) -> i64 {
    let raw_words: Vec<String> = raw
        .split_whitespace()
        .map(normalize_word)
        .filter(|word| !word.is_empty())
        .collect();
    let clean_words: Vec<String> = clean
        .split_whitespace()
        .map(normalize_word)
        .filter(|word| !word.is_empty())
        .collect();

    let mut previous: Vec<usize> = (0..=clean_words.len()).collect();
    for (raw_index, raw_word) in raw_words.iter().enumerate() {
        let mut current = vec![raw_index + 1; clean_words.len() + 1];
        for (clean_index, clean_word) in clean_words.iter().enumerate() {
            let substitution = previous[clean_index] + usize::from(raw_word != clean_word);
            let insertion = current[clean_index] + 1;
            let deletion = previous[clean_index + 1] + 1;
            current[clean_index + 1] = substitution.min(insertion).min(deletion);
        }
        previous = current;
    }
    previous[clean_words.len()] as i64
}

/// Lowercases and strips punctuation, keeping only alphanumeric characters.
fn normalize_word(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Computes current/longest streak over the (already zero-filled, ascending)
/// daily series. A day with `words > 0` counts as active.
fn compute_streak(daily: &[InsightsDay]) -> InsightsStreak {
    let active_days = daily.iter().filter(|d| d.words > 0).count() as i64;

    let mut current_days = 0i64;
    // The series is zero-filled and ends on today. A quiet today (the day
    // isn't over yet) must not zero a streak that's still alive through
    // yesterday — GitHub-style, the streak holds until the day ends. The
    // grace applies only to the trailing day (today) itself: if today is
    // empty it's skipped, and counting continues strictly from yesterday.
    let mut iter = daily.iter().rev();
    if let Some(today) = iter.next() {
        if today.words > 0 {
            current_days += 1;
        }
    }
    for day in iter {
        if day.words > 0 {
            current_days += 1;
        } else {
            break;
        }
    }

    let mut longest_days = 0i64;
    let mut longest_words = 0i64;
    let mut longest_started_on = None;
    let mut longest_ended_on = None;
    let mut run = 0i64;
    let mut run_words = 0i64;
    let mut run_start: Option<&str> = None;
    for day in daily {
        if day.words > 0 {
            if run == 0 {
                run_start = Some(day.day.as_str());
            }
            run += 1;
            run_words += day.words;
        } else {
            run = 0;
            run_words = 0;
            run_start = None;
        }
        if run > longest_days {
            longest_days = run;
            longest_words = run_words;
            longest_started_on = run_start.map(str::to_string);
            longest_ended_on = Some(day.day.clone());
        }
    }

    InsightsStreak {
        current_days,
        longest_days,
        longest_started_on,
        longest_ended_on,
        longest_words,
        active_days,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Local, TimeZone};

    fn test_db() -> Db {
        open(":memory:").expect("test db")
    }

    /// UTC-naive timestamp whose local representation lands on `days_ago`
    /// local calendar days at the given local wall-clock time.
    fn utc_for_local_day(days_ago: i64, hour: u32, minute: u32) -> String {
        let local_date = Local::now().date_naive() - Duration::days(days_ago);
        let wall = local_date
            .and_hms_opt(hour, minute, 0)
            .expect("valid wall time");
        let utc = Local
            .from_local_datetime(&wall)
            .single()
            .expect("local to utc")
            .naive_utc();
        utc.format("%Y-%m-%d %H:%M:%S").to_string()
    }

    fn local_day_string(days_ago: i64) -> String {
        (Local::now().date_naive() - Duration::days(days_ago))
            .format("%Y-%m-%d")
            .to_string()
    }

    fn insert_on_day(db: &Db, days_ago: i64, text: &str, words: i64, duration_ms: i64) -> i64 {
        let entry = insert_transcription_returning(
            db,
            text,
            text,
            words,
            duration_ms,
            "groq/whisper-large-v3-turbo",
            None,
            None,
        )
        .expect("insert transcription");
        {
            let conn = lock_conn(db).expect("lock");
            conn.execute(
                "UPDATE transcriptions SET created_at = ?1 WHERE id = ?2",
                params![utc_for_local_day(days_ago, 10, 0), entry.id],
            )
            .expect("backdate transcription");
        }
        entry.id
    }

    fn days_in(insights: &Insights) -> Vec<String> {
        insights.daily.iter().map(|d| d.day.clone()).collect()
    }

    fn set_context(db: &Db, transcription_id: i64, context_id: i64) {
        let conn = lock_conn(db).expect("lock");
        conn.execute(
            "UPDATE transcriptions SET context_id = ?1 WHERE id = ?2",
            params![context_id, transcription_id],
        )
        .expect("set context");
    }

    #[test]
    fn insights_timestamp_range_predicate_uses_created_at_index() {
        let db = test_db();
        let conn = lock_conn(&db).expect("lock");
        let plan: Vec<String> = conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM transcriptions
                 WHERE created_at >= ?1 AND created_at < ?2",
            )
            .expect("explain")
            .query_map(["2026-01-01 00:00:00", "2026-02-01 00:00:00"], |row| {
                row.get(3)
            })
            .expect("plan rows")
            .collect::<rusqlite::Result<_>>()
            .expect("plan");
        assert!(
            plan.iter()
                .any(|detail| detail.contains("idx_transcriptions_created_at")),
            "expected created_at range seek, got {plan:?}"
        );
    }

    #[test]
    fn context_filter_scopes_per_dictation_figures_but_not_lifetime_counters() {
        let db = test_db();
        let work = insert_context_returning(&db, "Work", None, None, None, None, false)
            .expect("context")
            .id;

        // 120 wpm, attributed to Work.
        let a = insert_on_day(&db, 1, "alpha bravo charlie delta", 4, 2_000);
        set_context(&db, a, work);
        // 60 wpm, attributed to Work.
        let b = insert_on_day(&db, 2, "echo foxtrot", 2, 2_000);
        set_context(&db, b, work);
        // Another context entirely, plus one pre-v18 row with no context at all.
        let other = insert_on_day(&db, 1, "golf hotel india juliet kilo", 5, 1_000);
        set_context(&db, other, 999);
        insert_on_day(&db, 1, "unattributed words here", 3, 1_000);

        let all = query_insights(&db, 30, None).expect("unscoped");
        assert_eq!(all.context_id, None);
        assert_eq!(all.totals.total_transcriptions, 4);
        assert_eq!(all.totals.words_in_range, 14);

        let scoped = query_insights(&db, 30, Some(work)).expect("scoped");
        assert_eq!(scoped.context_id, Some(work));
        assert_eq!(scoped.totals.total_transcriptions, 2);
        assert_eq!(scoped.totals.words_in_range, 6);
        // Average of each clip's own wpm: (120 + 60) / 2.
        assert!(
            (scoped.totals.avg_wpm - 90.0).abs() < 0.001,
            "{}",
            scoped.totals.avg_wpm
        );
        assert_eq!(scoped.totals.best_wpm, 120);
        // Daily buckets only carry this context's days.
        let scoped_words: i64 = scoped.daily.iter().map(|d| d.words).sum();
        assert_eq!(scoped_words, 6);
        // Vocabulary is scoped too — the other context's words must not leak in.
        assert!(scoped.words.top.iter().all(|w| w.word != "golf"));
        assert!(all.words.top.iter().any(|w| w.word == "golf"));

        // Lifetime counters have no context dimension and stay global.
        assert_eq!(
            scoped.cleanup.dictionary_fixes,
            all.cleanup.dictionary_fixes
        );
        assert_eq!(
            scoped.cleanup.auto_learned_terms,
            all.cleanup.auto_learned_terms
        );
        // total_words falls back to the context's own history rather than the
        // global lifetime figure.
        assert_eq!(scoped.totals.total_words, 6);
    }

    #[test]
    fn empty_db_returns_zero_payload_not_an_error() {
        let db = test_db();
        let insights = query_insights(&db, 30, None).expect("insights on empty db");

        assert_eq!(insights.range_days, 30);
        assert_eq!(insights.totals.total_words, 0);
        assert_eq!(insights.totals.total_transcriptions, 0);
        assert_eq!(insights.totals.total_speaking_ms, 0);
        assert_eq!(insights.totals.avg_words_per_transcription, 0);
        assert_eq!(insights.totals.avg_wpm, 0.0);
        assert_eq!(insights.totals.best_wpm, 0);
        assert_eq!(insights.totals.words_in_range, 0);
        assert_eq!(insights.totals.words_prev_range, 0);
        assert_eq!(insights.streak.current_days, 0);
        assert_eq!(insights.streak.longest_days, 0);
        assert_eq!(insights.streak.longest_started_on, None);
        assert_eq!(insights.streak.longest_ended_on, None);
        assert_eq!(insights.streak.longest_words, 0);
        assert_eq!(insights.streak.active_days, 0);
        assert_eq!(insights.daily.len(), 30);
        assert!(insights.daily.iter().all(|d| d.words == 0));
        assert_eq!(insights.hourly.len(), 24);
        assert!(insights.hourly.iter().all(|h| *h == 0));
        assert!(insights.providers.is_empty());
        assert_eq!(insights.cleanup.raw_words, 0);
        assert_eq!(insights.cleanup.clean_words, 0);
        assert_eq!(insights.cleanup.edits_applied, 0);
        assert_eq!(insights.cleanup.dictionary_fixes, 0);
        assert_eq!(insights.cleanup.auto_learned_terms, 0);
        assert!(insights.words.top.is_empty());
        assert_eq!(insights.words.unique_words, 0);
        assert_eq!(insights.words.longest_word, None);
        assert_eq!(insights.words.avg_word_length, 0.0);
    }

    #[test]
    fn all_time_range_spans_first_transcription_through_today() {
        let db = test_db();
        insert_on_day(&db, 5, "hello world", 2, 1000);
        insert_on_day(&db, 2, "more dictation", 2, 1000);

        let insights = query_insights(&db, 0, None).expect("insights all time");
        let expected = (0..=5).rev().map(local_day_string).collect::<Vec<_>>();
        assert_eq!(days_in(&insights), expected);
        assert_eq!(insights.range_days, 0);
        assert_eq!(insights.totals.words_in_range, 4);
        assert_eq!(insights.totals.words_prev_range, 0);
    }

    #[test]
    fn daily_zero_fills_a_gap_day_and_stays_ascending() {
        let db = test_db();
        insert_on_day(&db, 3, "day one", 2, 1000);
        insert_on_day(&db, 1, "day three", 3, 1500);

        let insights = query_insights(&db, 7, None).expect("insights");
        let expected = (0..7).rev().map(local_day_string).collect::<Vec<_>>();
        assert_eq!(days_in(&insights), expected);

        let gap = insights
            .daily
            .iter()
            .find(|d| d.day == local_day_string(2))
            .expect("gap day present");
        assert_eq!(gap.words, 0);
        assert_eq!(gap.transcriptions, 0);
        assert_eq!(gap.speaking_ms, 0);

        let active: Vec<_> = insights.daily.iter().filter(|d| d.words > 0).collect();
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].day, local_day_string(3));
        assert_eq!(active[0].words, 2);
        assert_eq!(active[1].day, local_day_string(1));
        assert_eq!(active[1].words, 3);
        assert_eq!(active[1].speaking_ms, 1500);
    }

    #[test]
    fn streak_calendar_retains_one_compact_year() {
        let db = test_db();
        insert_on_day(&db, 2, "recent activity", 3, 1_500);

        let insights = query_insights(&db, 7, None).expect("insights");
        assert_eq!(
            insights.daily.len(),
            7,
            "page data respects the selected range"
        );
        assert_eq!(
            insights.streak_daily.len(),
            365,
            "one compact year is retained"
        );
        assert_eq!(
            insights.streak_daily.last().map(|d| d.day.as_str()),
            Some(local_day_string(0).as_str()),
            "calendar ends today"
        );
    }

    #[test]
    fn streak_counts_a_three_day_run_followed_by_a_gap() {
        let db = test_db();
        insert_on_day(&db, 6, "run one", 2, 1000);
        insert_on_day(&db, 5, "run two", 2, 1000);
        insert_on_day(&db, 4, "run three", 4, 2000);

        let insights = query_insights(&db, 7, None).expect("insights");

        assert_eq!(insights.streak.longest_days, 3);
        assert_eq!(
            insights.streak.longest_started_on.as_deref(),
            Some(local_day_string(6).as_str())
        );
        assert_eq!(
            insights.streak.longest_ended_on.as_deref(),
            Some(local_day_string(4).as_str())
        );
        assert_eq!(insights.streak.longest_words, 8);
        assert_eq!(
            insights.streak.current_days, 0,
            "gap at the range end breaks the current streak"
        );
        assert_eq!(insights.streak.active_days, 3);
    }

    #[test]
    fn streak_survives_a_quiet_today() {
        let db = test_db();
        // Yesterday and the day before were active; today is silent (the day
        // isn't over yet) — the streak must still read as alive.
        insert_on_day(&db, 1, "yesterday", 2, 1000);
        insert_on_day(&db, 2, "two days ago", 2, 1000);

        let insights = query_insights(&db, 7, None).expect("insights");
        assert_eq!(insights.streak.current_days, 2);
    }

    #[test]
    fn streak_breaks_after_two_quiet_days() {
        let db = test_db();
        // Today and yesterday silent; the last activity is two days back.
        insert_on_day(&db, 2, "two days ago", 2, 1000);

        let insights = query_insights(&db, 7, None).expect("insights");
        assert_eq!(insights.streak.current_days, 0);
    }

    #[test]
    fn streak_stops_at_a_real_gap_even_when_today_is_active() {
        let db = test_db();
        // Today is active, but yesterday was a gap — the grace for a quiet
        // today must not swallow real historical gaps. Streak is just today.
        insert_on_day(&db, 0, "today", 2, 1000);
        insert_on_day(&db, 2, "two days ago", 2, 1000);

        let insights = query_insights(&db, 7, None).expect("insights");
        assert_eq!(insights.streak.current_days, 1);
    }

    #[test]
    fn transcription_just_before_local_midnight_buckets_to_the_local_day() {
        let db = test_db();
        // 23:50 local two days ago — within an hour of the local-day boundary.
        let entry = insert_transcription_returning(
            &db,
            "late night words",
            "late night words",
            3,
            1000,
            "groq/whisper-large-v3-turbo",
            None,
            None,
        )
        .expect("insert transcription");
        let created_at = utc_for_local_day(2, 23, 50);
        {
            let conn = lock_conn(&db).expect("lock");
            conn.execute(
                "UPDATE transcriptions SET created_at = ?1 WHERE id = ?2",
                params![created_at, entry.id],
            )
            .expect("set boundary timestamp");
        }

        let insights = query_insights(&db, 0, None).expect("insights");

        let expected_local_day = local_day_string(2);
        let bucket = insights
            .daily
            .iter()
            .find(|d| d.words > 0)
            .expect("transcription bucketed somewhere");
        assert_eq!(bucket.day, expected_local_day);

        // When the machine's local offset is non-zero, the UTC calendar day
        // of the stored timestamp must NOT be the bucket (proving the
        // 'localtime' conversion happened).
        let utc_day = created_at[..10].to_string();
        if utc_day != expected_local_day {
            assert!(
                !insights
                    .daily
                    .iter()
                    .any(|d| d.day == utc_day && d.words > 0),
                "must bucket to the local day, not the UTC one"
            );
        }
    }

    #[test]
    fn top_words_exclude_stopwords_and_sort_descending() {
        let db = test_db();
        let text = "the the the and and and apple banana banana cherry the";
        insert_transcription_returning(
            &db,
            text,
            text,
            11,
            1000,
            "groq/whisper-large-v3-turbo",
            None,
            None,
        )
        .expect("insert transcription");

        let insights = query_insights(&db, 7, None).expect("insights");

        let top = &insights.words.top;
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].word, "banana");
        assert_eq!(top[0].count, 2);
        assert_eq!(top[1].word, "apple");
        assert_eq!(top[1].count, 1);
        assert_eq!(top[2].word, "cherry");
        assert_eq!(top[2].count, 1);
        assert!(
            !top.iter().any(|w| w.word == "the" || w.word == "and"),
            "stopwords must not appear in top words"
        );
        assert_eq!(insights.words.unique_words, 3);
        assert_eq!(insights.words.longest_word.as_deref(), Some("banana"));
        assert!(insights.words.avg_word_length > 5.0 && insights.words.avg_word_length < 7.0);
    }

    #[test]
    fn streaming_metrics_preserve_word_rules_and_count_excluded_tokens() {
        let db = test_db();
        let conn = lock_conn(&db).expect("lock");
        conn.execute(
            "INSERT INTO transcriptions (raw_text, clean_text, words, spoken_words)
             VALUES (?1, ?1, 4, NULL)",
            ["re-enter reenter café café foo !!!"],
        )
        .expect("insert");
        let range = range_bounds(&conn, 0, None).expect("range");
        let (words, raw, clean, changed) =
            query_text_metrics(&conn, &range, None).expect("metrics");
        assert_eq!((raw, clean), (6, 6));
        assert_eq!(changed, 0);
        assert_eq!(words.unique_words, 3);
        assert_eq!(words.longest_word.as_deref(), Some("reenter"));
        assert_eq!(words.avg_word_length, 5.0);
        assert!(words
            .top
            .iter()
            .any(|word| word.word == "café" && word.count == 2));
        assert!(words
            .top
            .iter()
            .any(|word| word.word == "reenter" && word.count == 2));
    }

    #[test]
    fn top_words_strip_punctuation_and_drop_short_tokens() {
        let db = test_db();
        let text = "React, Vue! React? Vue. Svelte. i a um go";
        insert_transcription_returning(
            &db,
            text,
            text,
            10,
            1000,
            "groq/whisper-large-v3-turbo",
            None,
            None,
        )
        .expect("insert transcription");

        let insights = query_insights(&db, 7, None).expect("insights");

        let top = &insights.words.top;
        assert!(top.iter().any(|w| w.word == "react" && w.count == 2));
        assert!(top.iter().any(|w| w.word == "vue" && w.count == 2));
        assert!(top.iter().any(|w| w.word == "svelte" && w.count == 1));
        assert!(
            !top.iter()
                .any(|w| w.word == "i" || w.word == "a" || w.word == "um"),
            "sub-3-char and stopword tokens must be excluded"
        );
    }

    #[test]
    fn api_calls_round_trip_and_aggregate_per_model_and_task() {
        let db = test_db();
        let id = insert_on_day(&db, 1, "cost test", 2, 1000);
        let created_at = utc_for_local_day(1, 10, 0);

        let calls = build_api_calls(
            id,
            &created_at,
            "groq/whisper-large-v3-turbo;cleanup=groq/llama-3.3-70b-versatile",
            1234,
            "raw words",
            "clean words",
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].task, "transcription");
        assert_eq!(calls[0].audio_ms, 1234);
        assert_eq!(calls[1].task, "cleanup");
        assert_eq!(calls[1].input_chars, 9);
        assert_eq!(calls[1].output_chars, 11);

        insert_api_calls(&db, &calls).expect("insert api calls");

        let insights = query_insights(&db, 7, None).expect("insights");
        assert_eq!(insights.providers.len(), 2);
        let tx = insights
            .providers
            .iter()
            .find(|p| p.task == "transcription")
            .expect("transcription row");
        assert_eq!(tx.model, "whisper-large-v3-turbo");
        assert_eq!(tx.provider, "groq");
        assert_eq!(tx.calls, 1);
        assert_eq!(tx.audio_ms, 1234);
        let cleanup = insights
            .providers
            .iter()
            .find(|p| p.task == "cleanup")
            .expect("cleanup row");
        assert_eq!(cleanup.model, "llama-3.3-70b-versatile");
        assert_eq!(cleanup.calls, 1);
        assert_eq!(cleanup.input_chars, 9);
        assert_eq!(cleanup.output_chars, 11);
    }

    #[test]
    fn serde_field_names_match_the_frontend_contract_exactly() {
        let db = test_db();
        let insights = query_insights(&db, 30, None).expect("insights");
        let json = serde_json::to_value(&insights).expect("serialize insights");
        let object = json.as_object().expect("object");

        for key in [
            "range_days",
            "generated_at",
            "totals",
            "streak",
            "daily",
            "streak_daily",
            "history_started_on",
            "hourly",
            "providers",
            "cleanup",
            "words",
        ] {
            assert!(object.contains_key(key), "missing top-level key: {key}");
        }
        for key in [
            "total_words",
            "total_transcriptions",
            "total_speaking_ms",
            "avg_words_per_transcription",
            "avg_wpm",
            "best_wpm",
            "words_in_range",
            "words_prev_range",
        ] {
            assert!(
                object["totals"]
                    .as_object()
                    .expect("totals")
                    .contains_key(key),
                "missing totals key: {key}"
            );
        }
        for key in [
            "current_days",
            "longest_days",
            "longest_started_on",
            "longest_ended_on",
            "longest_words",
            "active_days",
        ] {
            assert!(
                object["streak"]
                    .as_object()
                    .expect("streak")
                    .contains_key(key),
                "missing streak key: {key}"
            );
        }
        for key in ["day", "words", "transcriptions", "speaking_ms"] {
            assert!(
                object["daily"]
                    .as_array()
                    .expect("daily")
                    .first()
                    .and_then(|d| d.as_object())
                    .expect("day object")
                    .contains_key(key),
                "missing daily key: {key}"
            );
        }
        assert_eq!(
            object["hourly"].as_array().expect("hourly").len(),
            24,
            "hourly must have exactly 24 entries"
        );
        for key in [
            "model",
            "provider",
            "task",
            "calls",
            "audio_ms",
            "input_chars",
            "output_chars",
        ] {
            assert!(
                object["providers"]
                    .as_array()
                    .expect("providers")
                    .first()
                    .and_then(|p| p.as_object())
                    .map(|o| o.contains_key(key))
                    .unwrap_or(true),
                "missing providers key: {key}"
            );
        }
        for key in [
            "raw_words",
            "clean_words",
            "edits_applied",
            "dictionary_fixes",
            "auto_learned_terms",
        ] {
            assert!(
                object["cleanup"]
                    .as_object()
                    .expect("cleanup")
                    .contains_key(key),
                "missing cleanup key: {key}"
            );
        }
        for key in ["top", "unique_words", "longest_word", "avg_word_length"] {
            assert!(
                object["words"]
                    .as_object()
                    .expect("words")
                    .contains_key(key),
                "missing words key: {key}"
            );
        }
        assert!(
            object["words"]["top"].as_array().expect("top").is_empty(),
            "top is an array"
        );
    }

    #[test]
    fn all_time_avg_wpm_matches_query_stats_on_the_same_data() {
        let db = test_db();
        insert_on_day(&db, 1, "two words", 2, 1000);
        insert_on_day(&db, 2, "also two", 2, 2000);

        let stats = query_stats(&db).expect("stats");
        let insights = query_insights(&db, 0, None).expect("insights");

        assert!(stats.avg_wpm > 0.0);
        assert!(
            (insights.totals.avg_wpm - stats.avg_wpm).abs() < 1e-9,
            "all-time insights avg_wpm ({}) must equal query_stats avg_wpm ({})",
            insights.totals.avg_wpm,
            stats.avg_wpm
        );
    }

    #[test]
    fn cleanup_headline_counts_changed_words_in_raw_vs_final_text() {
        let db = test_db();
        insert_transcription_returning(
            &db,
            "send the report to accouting tomorrow",
            "Send the report to accounting tomorrow, please.",
            7,
            1000,
            "test",
            None,
            None,
        )
        .expect("insert transcription");
        increment_lifetime_dictionary_fixes(&db, 3).expect("increment");

        let insights = query_insights(&db, 7, None).expect("insights");
        assert_eq!(insights.cleanup.dictionary_fixes, 3);
        assert_eq!(insights.cleanup.edits_applied, 2);

        increment_lifetime_dictionary_fixes(&db, 2).expect("increment again");
        let insights = query_insights(&db, 7, None).expect("insights");
        assert_eq!(insights.cleanup.dictionary_fixes, 5);
        assert_eq!(insights.cleanup.edits_applied, 2);
    }

    #[test]
    fn changed_word_count_ignores_case_and_punctuation() {
        assert_eq!(count_changed_words("Hello, world!", "hello world."), 0);
        assert_eq!(count_changed_words("send the report", "send report"), 1);
        assert_eq!(count_changed_words("send report", "send the report"), 1);
        assert_eq!(count_changed_words("send report", "send summary"), 1);
    }

    #[test]
    fn stopwords_stay_sorted_for_binary_search() {
        // `is_stopword` relies on binary_search over a sorted slice — a
        // mis-sorted STOPWORDS would silently fail lookups.
        assert!(STOPWORDS.windows(2).all(|w| w[0] < w[1]));
        // Spot-check membership works for both ends and a middle entry.
        assert!(is_stopword("a"));
        assert!(is_stopword("because"));
        assert!(is_stopword("yours"));
        assert!(!is_stopword("verenu"));
    }

    #[test]
    fn parse_api_usage_handles_all_pipeline_formats() {
        let parts = parse_api_usage("groq/whisper-large-v3-turbo/transcription");
        assert_eq!(
            parts.transcription_models,
            vec![("groq".to_string(), "whisper-large-v3-turbo".to_string())]
        );
        assert!(parts.cleanup_models.is_empty());

        let parts = parse_api_usage(
            "primary=groq/whisper-large-v3-turbo;secondary=openai/gpt-4o-transcribe;cleanup=groq/llama-3.3-70b-versatile",
        );
        assert_eq!(parts.transcription_models.len(), 2);
        assert_eq!(
            parts.transcription_models[0],
            ("groq".to_string(), "whisper-large-v3-turbo".to_string())
        );
        assert_eq!(
            parts.transcription_models[1],
            ("openai".to_string(), "gpt-4o-transcribe".to_string())
        );
        assert_eq!(
            parts.cleanup_models,
            vec![("groq".to_string(), "llama-3.3-70b-versatile".to_string())]
        );

        let parts = parse_api_usage("garbage");
        assert!(parts.transcription_models.is_empty());
        assert!(parts.cleanup_models.is_empty());
    }

    #[test]
    fn parse_api_usage_strips_transcription_suffix_in_primary_branch() {
        // The primary= branch must normalize model names exactly like the
        // single-provider fallback — a trailing /transcription must not leak
        // into the stored model key.
        let parts = parse_api_usage("primary=groq/whisper-large-v3-turbo/transcription");
        assert_eq!(
            parts.transcription_models,
            vec![("groq".to_string(), "whisper-large-v3-turbo".to_string())]
        );
    }
}
