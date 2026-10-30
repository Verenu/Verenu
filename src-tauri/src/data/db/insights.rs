use std::collections::HashMap;

use anyhow::Result;
use chrono::{Duration, Local, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Insights {
    pub context_id: Option<i64>,
    pub range_days: i64,
    pub generated_at: String,
    pub totals: InsightsTotals,
    pub streak: InsightsStreak,
    pub daily: Vec<InsightsDay>,
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
    pub vocabulary_replacements: i64,
    pub snippet_expansions: i64,
    pub activity_incomplete: bool,
    pub activity_tracked_dictations: i64,
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
}

#[derive(Debug)]
struct CleanupLifetime {
    dictionary_fixes: i64,
    auto_learned_terms: i64,
}

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
        activity,
    ) = {
        let conn = lock_conn(db)?;
        let history_started_on = history_started_on_conn(&conn, context_id)?;
        let range = range_bounds(days, history_started_on.as_deref())?;
        let previous = previous_range(days);
        let totals = query_totals(&conn, &range, previous.as_ref(), context_id)?;
        let lifetime_range = range_bounds(0, history_started_on.as_deref())?;
        let aggregate_start = range.start_day.min(lifetime_range.start_day);
        let aggregate_range = make_date_range(aggregate_start, range.end_day);
        let aggregate_daily = query_daily(&conn, &aggregate_range, context_id)?;
        let daily = slice_daily(&aggregate_daily, aggregate_start, &range);
        let lifetime_streak_daily = slice_daily(&aggregate_daily, aggregate_start, &lifetime_range);
        let streak_daily = compact_streak_daily(&lifetime_streak_daily);
        let hourly = query_hourly(&conn, &range, context_id)?;
        let providers = query_providers(&conn, &range, context_id)?;
        let (words, raw_words, clean_words, changed_words) =
            query_text_metrics(&conn, &range, context_id)?;
        let cleanup_lifetime = query_cleanup_lifetime(&conn, context_id)?;
        let activity = query_cleanup_activity(&conn, &range, context_id)?;

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
            activity,
        )
    };

    let cleanup = InsightsCleanup {
        raw_words,
        clean_words,
        edits_applied: changed_words,
        dictionary_fixes: cleanup_lifetime.dictionary_fixes,
        auto_learned_terms: cleanup_lifetime.auto_learned_terms,
        vocabulary_replacements: activity.0,
        snippet_expansions: activity.1,
        activity_incomplete: activity.2 || totals.total_transcriptions > activity.4,
        activity_tracked_dictations: activity.3,
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

struct ApiUsageParts {
    transcription_models: Vec<(String, String)>,
    cleanup_models: Vec<(String, String)>,
}

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

fn history_started_on_conn(conn: &Connection, context_id: Option<i64>) -> Result<Option<String>> {
    let sql = if context_id.is_some() {
        "SELECT MIN(day) FROM transcription_context_daily_stats
         WHERE context_uuid = (SELECT uuid FROM contexts WHERE id = ?1)"
    } else {
        "SELECT MIN(day) FROM transcription_daily_stats WHERE ?1 IS NULL"
    };
    conn.prepare_cached(sql)?
        .query_row(params![context_id], |row| row.get(0))
        .map_err(Into::into)
}

fn range_bounds(days: i64, history_started_on: Option<&str>) -> Result<DateRange> {
    let today = Local::now().date_naive();
    let start_day = if days > 0 {
        today - Duration::days((days - 1).max(0))
    } else {
        history_started_on
            .map(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d"))
            .transpose()?
            .unwrap_or(today)
    };
    Ok(make_date_range(start_day, today))
}

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
    DateRange { start_day, end_day }
}

fn query_totals(
    conn: &Connection,
    range: &DateRange,
    previous: Option<&DateRange>,
    context_id: Option<i64>,
) -> Result<InsightsTotals> {
    let context_uuid: Option<String> = match context_id {
        Some(id) => conn
            .query_row(
                "SELECT uuid FROM contexts WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?,
        None => None,
    };
    if context_id.is_some() && context_uuid.is_none() {
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
    }

    let source = if context_id.is_none() {
        "transcription_daily_stats WHERE ?5 IS NULL"
    } else {
        "transcription_context_daily_stats WHERE context_uuid = ?5"
    };
    let previous_start = previous.map(|r| r.start_day.format("%Y-%m-%d").to_string());
    let previous_end = previous.map(|r| r.end_day.format("%Y-%m-%d").to_string());
    let sql = format!("SELECT
       COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN total_transcriptions ELSE 0 END), 0),
       COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN total_words ELSE 0 END), 0),
       COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN speaking_ms ELSE 0 END), 0),
       COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN wpm_sum ELSE 0 END), 0),
       COALESCE(SUM(CASE WHEN day >= ?1 AND day <= ?2 THEN wpm_count ELSE 0 END), 0),
       COALESCE(MAX(CASE WHEN day >= ?1 AND day <= ?2 THEN best_wpm ELSE 0 END), 0),
       COALESCE(SUM(CASE WHEN ?3 IS NOT NULL AND day >= ?3 AND day <= ?4 THEN total_words ELSE 0 END), 0)
     FROM {source}");
    let (
        total_transcriptions,
        words_in_range,
        total_speaking_ms,
        wpm_sum,
        wpm_count,
        best_wpm,
        words_prev_range,
    ): (i64, i64, i64, f64, i64, f64, i64) = conn.query_row(
        &sql,
        params![
            range.start_day.format("%Y-%m-%d").to_string(),
            range.end_day.format("%Y-%m-%d").to_string(),
            previous_start,
            previous_end,
            context_uuid
        ],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        },
    )?;
    let total_words: i64 = if context_id.is_none() {
        conn.query_row(
            "SELECT COALESCE((SELECT total_words FROM lifetime_stats WHERE id = 1), 0)
                  + COALESCE((SELECT SUM(total_words) FROM combined_device_counters), 0)",
            [],
            |r| r.get(0),
        )?
    } else {
        conn.query_row(
            "SELECT COALESCE(SUM(total_words), 0) FROM transcription_context_daily_stats WHERE context_uuid = ?1",
            params![context_uuid], |r| r.get(0),
        )?
    };
    Ok(InsightsTotals {
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
    })
}

fn query_daily(
    conn: &Connection,
    range: &DateRange,
    context_id: Option<i64>,
) -> Result<Vec<InsightsDay>> {
    let mut per_day: HashMap<String, (i64, i64, i64)> = HashMap::new();
    {
        let sql = if context_id.is_none() {
            "SELECT day, total_words, total_transcriptions, speaking_ms
               FROM transcription_daily_stats
              WHERE day >= ?1 AND day <= ?2 AND ?3 IS NULL"
        } else {
            "SELECT day, total_words, total_transcriptions, speaking_ms
               FROM transcription_context_daily_stats
              WHERE day >= ?1 AND day <= ?2
                AND context_uuid = (SELECT uuid FROM contexts WHERE id = ?3)"
        };
        let mut stmt = conn.prepare(sql)?;
        let start_bound = range.start_day.format("%Y-%m-%d").to_string();
        let end_bound = range.end_day.format("%Y-%m-%d").to_string();
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

fn query_hourly(conn: &Connection, range: &DateRange, context_id: Option<i64>) -> Result<Vec<i64>> {
    let mut hourly = vec![0i64; 24];
    let mut stmt = conn.prepare(
        "SELECT COALESCE(activity_hour, CAST(strftime('%H', created_at, 'localtime') AS INTEGER)), SUM(words)
         FROM transcriptions
         WHERE COALESCE(activity_day, date(created_at, 'localtime')) >= ?1 AND COALESCE(activity_day, date(created_at, 'localtime')) <= ?2
           AND NOT EXISTS(SELECT 1 FROM backup_held_history h WHERE h.uuid = transcriptions.uuid)
           AND NOT EXISTS(SELECT 1 FROM backup_held_hourly_receipts h WHERE h.uuid = transcriptions.uuid)
           AND (?3 IS NULL OR context_id = ?3)
         GROUP BY 1",
    )?;
    let rows = stmt.query_map(params![range.start_day.to_string(), range.end_day.to_string(), context_id], |r| {
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

fn query_providers(
    conn: &Connection,
    range: &DateRange,
    context_id: Option<i64>,
) -> Result<Vec<InsightsProviderUsage>> {
    let mut stmt = conn.prepare(
        "SELECT a.model, a.provider, a.task, COUNT(*),
                COALESCE(SUM(a.audio_ms), 0),
                COALESCE(SUM(a.input_chars), 0),
                COALESCE(SUM(a.output_chars), 0)
         FROM api_calls a
         LEFT JOIN transcriptions t ON t.id = a.transcription_id
         WHERE COALESCE(a.activity_day, date(a.created_at, 'localtime')) >= ?1 AND COALESCE(a.activity_day, date(a.created_at, 'localtime')) <= ?2
           AND NOT EXISTS(SELECT 1 FROM backup_held_calls h WHERE h.uuid = a.uuid)
           AND (?3 IS NULL OR t.context_id = ?3)
         GROUP BY a.model, a.provider, a.task
         ORDER BY COUNT(*) DESC, a.model ASC",
    )?;
    let rows = stmt.query_map(
        params![range.start_day.to_string(), range.end_day.to_string(), context_id],
        provider_usage_from_row,
    )?;
    let mut by_key: HashMap<(String, String, String), InsightsProviderUsage> = HashMap::new();
    for row in rows {
        merge_provider_usage(&mut by_key, row?);
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
        provider_usage_from_row,
    )?;
    for row in rows {
        merge_provider_usage(&mut by_key, row?);
    }
    let mut usage: Vec<_> = by_key.into_values().collect();
    usage.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.model.cmp(&b.model)));
    Ok(usage)
}

fn provider_usage_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<InsightsProviderUsage> {
    Ok(InsightsProviderUsage {
        model: r.get(0)?,
        provider: r.get(1)?,
        task: r.get(2)?,
        calls: r.get(3)?,
        audio_ms: r.get(4)?,
        input_chars: r.get(5)?,
        output_chars: r.get(6)?,
    })
}

fn merge_provider_usage(
    by_key: &mut HashMap<(String, String, String), InsightsProviderUsage>,
    value: InsightsProviderUsage,
) {
    let key = (
        value.model.clone(),
        value.provider.clone(),
        value.task.clone(),
    );
    by_key
        .entry(key)
        .and_modify(|entry| {
            entry.calls += value.calls;
            entry.audio_ms += value.audio_ms;
            entry.input_chars += value.input_chars;
            entry.output_chars += value.output_chars;
        })
        .or_insert(value);
}

fn query_cleanup_lifetime(conn: &Connection, context_id: Option<i64>) -> Result<CleanupLifetime> {
    let dictionary_fixes: i64 = match context_id {
        None => conn.query_row(
            "SELECT COALESCE((SELECT dictionary_fixes FROM lifetime_stats WHERE id = 1), 0)
                  + COALESCE((SELECT SUM(dictionary_fixes) FROM combined_device_counters), 0)",
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
        "SELECT COUNT(*) FROM dictionary d WHERE
           EXISTS (SELECT 1 FROM dictionary_corrections c WHERE c.dictionary_id = d.id AND c.auto_learned = 1)
           OR (d.auto_learned = 1 AND NOT EXISTS (SELECT 1 FROM dictionary_corrections c WHERE c.dictionary_id = d.id))",
        [],
        |r| r.get(0),
    )?;
    Ok(CleanupLifetime {
        dictionary_fixes,
        auto_learned_terms,
    })
}

fn query_cleanup_activity(conn: &Connection, range: &DateRange, context_id: Option<i64>) -> Result<(i64, i64, bool, i64, i64)> {
    conn.query_row(
        "SELECT COALESCE(SUM(vocabulary_replacements), 0), COALESCE(SUM(snippet_expansions), 0),
                COALESCE(MAX(vocabulary_replacements IS NULL OR snippet_expansions IS NULL), 0),
                COUNT(CASE WHEN vocabulary_replacements IS NOT NULL AND snippet_expansions IS NOT NULL THEN 1 END),
                COUNT(CASE WHEN NOT EXISTS (SELECT 1 FROM backup_held_history h WHERE h.uuid = transcriptions.uuid) THEN 1 END)
           FROM transcriptions WHERE (activity_day BETWEEN ?1 AND ?2
            OR (activity_day IS NULL AND date(created_at, 'localtime') BETWEEN ?1 AND ?2))
            AND (?3 IS NULL OR context_id = ?3)",
        params![range.start_day.to_string(), range.end_day.to_string(), context_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).map_err(Into::into)
}

fn query_text_metrics(
    conn: &Connection,
    range: &DateRange,
    context_id: Option<i64>,
) -> Result<(InsightsWords, i64, i64, i64)> {
    let mut stmt = conn.prepare(
        "SELECT raw_text, clean_text FROM transcriptions
         WHERE (activity_day BETWEEN ?1 AND ?2
           OR (activity_day IS NULL AND date(created_at, 'localtime') BETWEEN ?1 AND ?2))
           AND (?3 IS NULL OR context_id = ?3)",
    )?;
    let mut rows = stmt.query(params![range.start_day.to_string(), range.end_day.to_string(), context_id])?;
    let mut counts: HashMap<String, i64> = HashMap::new();
    let mut longest: Option<String> = None;
    let mut longest_len = 0;
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
            if !token.chars().any(char::is_alphanumeric) {
                continue;
            }
            clean_words += 1;
            let normalized = normalize_word(token);
            if normalized.is_empty() {
                continue;
            }
            let char_len = normalized.chars().count();
            if char_len < MIN_WORD_CHARS || is_stopword(&normalized) {
                continue;
            }
            if char_len > longest_len {
                longest = Some(normalized.clone());
                longest_len = char_len;
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
    let compare = |a: &InsightsWordCount, b: &InsightsWordCount| {
        b.count.cmp(&a.count).then_with(|| a.word.cmp(&b.word))
    };
    if top.len() > TOP_WORDS_LIMIT {
        top.select_nth_unstable_by(TOP_WORDS_LIMIT, compare);
    }
    top.truncate(TOP_WORDS_LIMIT);
    top.sort_by(compare);
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

    let prefix = raw_words.iter().zip(&clean_words)
        .take_while(|(a, b)| a == b).count();
    let raw_words = &raw_words[prefix..];
    let clean_words = &clean_words[prefix..];
    let suffix = raw_words.iter().rev().zip(clean_words.iter().rev())
        .take_while(|(a, b)| a == b).count();
    let raw_words = &raw_words[..raw_words.len() - suffix];
    let clean_words = &clean_words[..clean_words.len() - suffix];
    let (longer, shorter) = if raw_words.len() >= clean_words.len() {
        (raw_words, clean_words)
    } else {
        (clean_words, raw_words)
    };
    if shorter.is_empty() {
        return longer.len() as i64;
    }

    let mut distances: Vec<usize> = (0..=shorter.len()).collect();
    for (row, word) in longer.iter().enumerate() {
        let mut diagonal = distances[0];
        distances[0] = row + 1;
        for (column, other) in shorter.iter().enumerate() {
            let above = distances[column + 1];
            distances[column + 1] = (diagonal + usize::from(word != other))
                .min(distances[column] + 1)
                .min(above + 1);
            diagonal = above;
        }
    }
    distances[shorter.len()] as i64
}

fn normalize_word(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn compute_streak(daily: &[InsightsDay]) -> InsightsStreak {
    let active_days = daily.iter().filter(|d| d.words > 0).count() as i64;

    let mut current_days = 0i64;
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
    fn insights_activity_day_range_uses_index() {
        let db = test_db();
        let conn = lock_conn(&db).unwrap();
        let plan: Vec<String> = conn.prepare("EXPLAIN QUERY PLAN SELECT raw_text, clean_text FROM transcriptions WHERE (activity_day BETWEEN ?1 AND ?2 OR (activity_day IS NULL AND date(created_at, 'localtime') BETWEEN ?1 AND ?2)) AND (?3 IS NULL OR context_id = ?3)").unwrap()
            .query_map(params!["2026-01-01", "2026-02-01", Option::<i64>::None], |row| row.get(3)).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap();
        assert!(plan.iter().any(|detail| detail.contains("idx_transcriptions_activity_day")), "{plan:?}");
    }

    #[test]
    fn context_filter_scopes_per_dictation_figures_but_not_lifetime_counters() {
        let db = test_db();
        let work = insert_context_returning(&db, "Work", None, None, None, None, false)
            .expect("context")
            .id;

        let a = insert_on_day(&db, 1, "alpha bravo charlie delta", 4, 2_000);
        set_context(&db, a, work);
        let b = insert_on_day(&db, 2, "echo foxtrot", 2, 2_000);
        set_context(&db, b, work);
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
        assert!(
            (scoped.totals.avg_wpm - 90.0).abs() < 0.001,
            "{}",
            scoped.totals.avg_wpm
        );
        assert_eq!(scoped.totals.best_wpm, 120);
        let scoped_words: i64 = scoped.daily.iter().map(|d| d.words).sum();
        assert_eq!(scoped_words, 6);
        assert!(scoped.words.top.iter().all(|w| w.word != "golf"));
        assert!(all.words.top.iter().any(|w| w.word == "golf"));

        assert_eq!(
            scoped.cleanup.dictionary_fixes,
            all.cleanup.dictionary_fixes
        );
        assert_eq!(
            scoped.cleanup.auto_learned_terms,
            all.cleanup.auto_learned_terms
        );
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
        insert_on_day(&db, 1, "day three", 2, 1500);

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
        assert_eq!(active[1].words, 2);
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
        insert_on_day(&db, 4, "run three", 2, 2000);

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
        assert_eq!(insights.streak.longest_words, 6);
        assert_eq!(
            insights.streak.current_days, 0,
            "gap at the range end breaks the current streak"
        );
        assert_eq!(insights.streak.active_days, 3);
    }

    #[test]
    fn streak_survives_a_quiet_today() {
        let db = test_db();
        insert_on_day(&db, 1, "yesterday", 2, 1000);
        insert_on_day(&db, 2, "two days ago", 2, 1000);

        let insights = query_insights(&db, 7, None).expect("insights");
        assert_eq!(insights.streak.current_days, 2);
    }

    #[test]
    fn streak_breaks_after_two_quiet_days() {
        let db = test_db();
        insert_on_day(&db, 2, "two days ago", 2, 1000);

        let insights = query_insights(&db, 7, None).expect("insights");
        assert_eq!(insights.streak.current_days, 0);
    }

    #[test]
    fn streak_stops_at_a_real_gap_even_when_today_is_active() {
        let db = test_db();
        insert_on_day(&db, 0, "today", 2, 1000);
        insert_on_day(&db, 2, "two days ago", 2, 1000);

        let insights = query_insights(&db, 7, None).expect("insights");
        assert_eq!(insights.streak.current_days, 1);
    }

    #[test]
    fn transcription_just_before_local_midnight_buckets_to_the_local_day() {
        let db = test_db();
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
             VALUES ('synthetic', ?1, 4, NULL)",
            ["re-enter reenter café café foo !!!"],
        )
        .expect("insert");
        let history_started_on = history_started_on_conn(&conn, None).expect("history start");
        let range = range_bounds(0, history_started_on.as_deref()).expect("range");
        let (words, raw, clean, changed) =
            query_text_metrics(&conn, &range, None).expect("metrics");
        assert_eq!((raw, clean), (1, 5));
        assert_eq!(changed, 5);
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
    fn top_words_limit_preserves_frequency_and_alphabetical_ties() {
        let db = test_db();
        let text = "zebra zebra zebra kiwi kiwi kiwi mango mango apple banana cherry coconut date fig grape lemon orange peach pear plum";
        insert_transcription_returning(&db, text, text, 0, 1000, "test", None, None)
            .expect("insert");
        let insights = query_insights(&db, 7, None).expect("insights");
        let top: Vec<_> = insights.words.top.iter().map(|entry| (entry.word.as_str(), entry.count)).collect();
        assert_eq!(top, vec![
            ("kiwi", 3), ("zebra", 3), ("mango", 2), ("apple", 1),
            ("banana", 1), ("cherry", 1), ("coconut", 1), ("date", 1),
            ("fig", 1), ("grape", 1), ("lemon", 1), ("orange", 1),
        ]);
        assert_eq!(insights.words.unique_words, 15);
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
    fn cleanup_activity_respects_range_and_context_and_marks_legacy_history() {
        let db = test_db();
        let work = insert_context_returning(&db, "Work", None, None, None, None, false).expect("context");
        let other = insert_context_returning(&db, "Other", None, None, None, None, false).expect("context");
        let recent = insert_on_day(&db, 0, "synthetic words", 2, 1000);
        let legacy = insert_on_day(&db, 0, "older synthetic words", 3, 1000);
        let outside = insert_on_day(&db, 10, "outside range", 2, 1000);
        let elsewhere = insert_on_day(&db, 0, "other context", 2, 1000);
        {
            let conn = lock_conn(&db).unwrap();
            for (id, context, fixes, snippets) in [
                (recent, work.id, Some(3), Some(2)),
                (legacy, work.id, None, None),
                (outside, work.id, Some(20), Some(20)),
                (elsewhere, other.id, Some(4), Some(5)),
            ] {
                conn.execute("UPDATE transcriptions SET context_id=?1, vocabulary_replacements=?2, snippet_expansions=?3 WHERE id=?4", params![context, fixes, snippets, id]).unwrap();
            }
        }
        let scoped = query_insights(&db, 7, Some(work.id)).unwrap().cleanup;
        assert_eq!((scoped.vocabulary_replacements, scoped.snippet_expansions), (3, 2));
        assert!(scoped.activity_incomplete);
        let all = query_insights(&db, 7, None).unwrap().cleanup;
        assert_eq!((all.vocabulary_replacements, all.snippet_expansions), (7, 7));
        assert!(all.activity_incomplete);
        let other = query_insights(&db, 7, Some(other.id)).unwrap().cleanup;
        assert_eq!((other.vocabulary_replacements, other.snippet_expansions), (4, 5));
        assert!(!other.activity_incomplete);
    }

    #[test]
    fn learned_terms_include_corrections_added_to_existing_manual_terms() {
        let db = test_db();
        let work = insert_context_returning(&db, "Work", None, None, None, None, false).unwrap();
        let other = insert_context_returning(&db, "Other", None, None, None, None, false).unwrap();
        insert_dictionary_entry_returning(&db, "Kubernetes", Some("kubernetes typo"), Some(work.id)).unwrap();
        insert_dictionary_entry_auto_learned_for_context(&db, other.id, "Kubernetes", Some("kubernetez"), "high").unwrap();
        assert_eq!(query_insights(&db, 7, None).unwrap().cleanup.auto_learned_terms, 1);
    }

    #[test]
    fn cleanup_activity_marks_pruned_history_as_incomplete() {
        let db = test_db();
        let entry = insert_on_day(&db, 10, "synthetic words", 2, 1000);
        lock_conn(&db).unwrap().execute("UPDATE transcriptions SET vocabulary_replacements=2, snippet_expansions=1 WHERE id=?1", [entry]).unwrap();
        assert_eq!(prune_transcriptions_older_than(&db, 1).unwrap(), 1);
        let cleanup = query_insights(&db, 30, None).unwrap().cleanup;
        assert!(cleanup.activity_incomplete);
        assert_eq!(cleanup.activity_tracked_dictations, 0);
    }

    #[test]
    fn cleanup_activity_uses_preserved_day_for_imported_history() {
        let db = test_db();
        let entry = insert_on_day(&db, 10, "synthetic raw", 2, 1000);
        let today = local_day_string(0);
        lock_conn(&db).unwrap().execute("UPDATE transcriptions SET activity_day=?1, vocabulary_replacements=3, snippet_expansions=2 WHERE id=?2", params![today, entry]).unwrap();
        let cleanup = query_insights(&db, 1, None).unwrap().cleanup;
        assert_eq!((cleanup.vocabulary_replacements, cleanup.snippet_expansions), (3, 2));
        assert_eq!(cleanup.raw_words, 2);
    }

    #[test]
    fn learned_term_edited_to_manual_is_not_counted_as_learned() {
        let db = test_db();
        insert_dictionary_entry_auto_learned_for_context(&db, EVERYWHERE_CONTEXT_ID, "Kubernetes", Some("kubernetez"), "high").unwrap();
        let entry = query_dictionary_for_context(&db, EVERYWHERE_CONTEXT_ID).unwrap().into_iter().find(|entry| entry.term == "Kubernetes").unwrap();
        update_dictionary_entry_for_context(&db, EVERYWHERE_CONTEXT_ID, entry.id, "Kubernetes", Some("kubernetez")).unwrap();
        assert_eq!(query_insights(&db, 7, None).unwrap().cleanup.auto_learned_terms, 0);
    }

    #[test]
    fn changed_word_count_ignores_case_and_punctuation() {
        assert_eq!(count_changed_words("Hello, world!", "hello world."), 0);
        assert_eq!(count_changed_words("send the report", "send report"), 1);
        assert_eq!(count_changed_words("send report", "send the report"), 1);
        assert_eq!(count_changed_words("send report", "send summary"), 1);
    }

    #[test]
    fn changed_word_count_matches_full_matrix_for_repeated_words() {
        let mut sequences = vec![Vec::<&str>::new()];
        for length in 1..=4 {
            for bits in 0..(1 << length) {
                sequences.push((0..length).map(|bit| if bits & (1 << bit) == 0 { "alpha" } else { "beta" }).collect());
            }
        }
        for raw in &sequences {
            for clean in &sequences {
                let mut matrix = vec![vec![0; clean.len() + 1]; raw.len() + 1];
                for (row, values) in matrix.iter_mut().enumerate() {
                    values[0] = row;
                }
                for (column, value) in matrix[0].iter_mut().enumerate() {
                    *value = column;
                }
                for row in 1..=raw.len() {
                    for column in 1..=clean.len() {
                        matrix[row][column] = (matrix[row - 1][column - 1] + usize::from(raw[row - 1] != clean[column - 1]))
                            .min(matrix[row - 1][column] + 1)
                            .min(matrix[row][column - 1] + 1);
                    }
                }
                assert_eq!(count_changed_words(&raw.join(" "), &clean.join(" ")), matrix[raw.len()][clean.len()] as i64);
            }
        }
        assert_eq!(count_changed_words("café, !!! 世界", "CAFÉ 世界!"), 0);
    }

    #[test]
    fn stopwords_stay_sorted_for_binary_search() {
        assert!(STOPWORDS.windows(2).all(|w| w[0] < w[1]));
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
        let parts = parse_api_usage("primary=groq/whisper-large-v3-turbo/transcription");
        assert_eq!(
            parts.transcription_models,
            vec![("groq".to_string(), "whisper-large-v3-turbo".to_string())]
        );
    }
}
