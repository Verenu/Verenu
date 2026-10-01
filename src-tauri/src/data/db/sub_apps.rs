//! Sub-apps: a place inside an app (a Discord server, a VS Code project)
//! identified by the app plus a window-title rule. They form their own list
//! and can each be assigned to at most one Context; only assigned sub-apps
//! take part in resolution, which checks them before websites and apps.
//!
//! Only the user-approved title pattern is stored. Captured window titles are
//! never persisted or logged; they can contain private document names.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::*;

pub const SUB_APP_LABEL_CHAR_LIMIT: usize = 60;
pub const SUB_APP_PATTERN_CHAR_LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleMatchMode {
    Contains,
    StartsWith,
    Equals,
}

impl TitleMatchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::StartsWith => "starts_with",
            Self::Equals => "equals",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "contains" => Ok(Self::Contains),
            "starts_with" => Ok(Self::StartsWith),
            "equals" => Ok(Self::Equals),
            other => anyhow::bail!("Unknown title match mode '{other}'"),
        }
    }

    /// Higher wins when several rules match the same window.
    fn specificity(self) -> u8 {
        match self {
            Self::Equals => 3,
            Self::StartsWith => 2,
            Self::Contains => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSubApp {
    pub id: i64,
    pub uuid: String,
    /// `None` while the sub-app sits unassigned in the sub-app list.
    pub context_id: Option<i64>,
    pub executable: String,
    pub app_name: Option<String>,
    pub label: String,
    /// Icon key from the shared context icon set, or `None` for the app icon.
    pub icon: Option<String>,
    pub title_pattern: String,
    pub match_mode: TitleMatchMode,
    pub platform: Option<String>,
    pub created_at: String,
}

/// Case-insensitive, whitespace-collapsed form used for every comparison.
pub fn normalize_title(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn title_rule_matches(pattern: &str, mode: TitleMatchMode, title: &str) -> bool {
    let pattern = normalize_title(pattern);
    if pattern.is_empty() {
        return false;
    }
    let title = normalize_title(title);
    match mode {
        TitleMatchMode::Contains => title.contains(&pattern),
        TitleMatchMode::StartsWith => title.starts_with(&pattern),
        TitleMatchMode::Equals => title == pattern,
    }
}

/// Suggests a stable pattern from a captured title: drops unread counters
/// ("(3) ", "[12] ", "• "), unsaved markers ("● ", leading/trailing "*"),
/// and a trailing " - App" / " — App" / " | App" suffix naming the app.
pub fn propose_title_pattern(title: &str, app_name: Option<&str>) -> String {
    let mut value = title.split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        let trimmed = value.trim_start();
        let stripped = strip_counter_prefix(trimmed)
            .or_else(|| {
                ["• ", "● ", "* ", "*"]
                    .iter()
                    .find_map(|marker| trimmed.strip_prefix(marker))
            })
            .map(str::to_string);
        match stripped {
            Some(next) if next.len() < value.len() => value = next,
            _ => break,
        }
    }
    value = value.trim_end_matches('*').trim().to_string();
    if let Some(app) = app_name.map(normalize_title).filter(|app| !app.is_empty()) {
        for separator in [" - ", " — ", " – ", " | "] {
            if let Some(index) = value.rfind(separator) {
                let suffix = normalize_title(&value[index + separator.len()..]);
                if !suffix.is_empty() && (app.contains(&suffix) || suffix.contains(&app)) {
                    value.truncate(index);
                    break;
                }
            }
        }
    }
    value.trim().chars().take(SUB_APP_PATTERN_CHAR_LIMIT).collect()
}

fn strip_counter_prefix(value: &str) -> Option<&str> {
    let (open, close) = match value.chars().next()? {
        '(' => ('(', ')'),
        '[' => ('[', ']'),
        _ => return None,
    };
    let rest = value.strip_prefix(open)?;
    let end = rest.find(close)?;
    let inner = &rest[..end];
    (!inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit() || c == '+'))
        .then(|| rest[end + 1..].trim_start())
}

fn normalize_pattern(pattern: &str) -> Result<String> {
    let pattern = pattern.split_whitespace().collect::<Vec<_>>().join(" ");
    let pattern = require_nonempty_trimmed("Title pattern", &pattern)?;
    validate_char_limit("Title pattern", &pattern, SUB_APP_PATTERN_CHAR_LIMIT)?;
    Ok(pattern)
}

fn normalize_label(label: &str) -> Result<String> {
    let label = require_nonempty_trimmed("Sub-app name", label)?;
    validate_char_limit("Sub-app name", &label, SUB_APP_LABEL_CHAR_LIMIT)?;
    Ok(label)
}

fn sub_app_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContextSubApp> {
    let mode: String = row.get(8)?;
    Ok(ContextSubApp {
        id: row.get(0)?,
        uuid: row.get(1)?,
        context_id: row.get(2)?,
        executable: row.get(3)?,
        app_name: row.get(4)?,
        label: row.get(5)?,
        icon: row.get(6)?,
        title_pattern: row.get(7)?,
        match_mode: TitleMatchMode::parse(&mode).unwrap_or(TitleMatchMode::Contains),
        platform: row.get(9)?,
        created_at: row.get(10)?,
    })
}

const SUB_APP_COLUMNS: &str = "id, uuid, context_id, executable, app_name, label, icon, title_pattern, match_mode, platform, created_at";

/// Every sub-app visible on this device (assigned or not). Same visibility
/// rule as app targets: untagged or tagged for this platform.
pub fn query_sub_apps(db: &Db) -> Result<Vec<ContextSubApp>> {
    let conn = lock_conn(db)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {SUB_APP_COLUMNS} FROM context_sub_apps
          WHERE (?1 IS NULL OR platform IS NULL OR platform = ?1)
            AND executable NOT LIKE '?::%'
          ORDER BY label COLLATE NOCASE ASC"
    ))?;
    let rows = stmt
        .query_map(params![current_platform_tag()], sub_app_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn query_sub_app_conn(conn: &rusqlite::Connection, id: i64) -> Result<ContextSubApp> {
    conn.query_row(
        &format!("SELECT {SUB_APP_COLUMNS} FROM context_sub_apps WHERE id = ?1"),
        params![id],
        sub_app_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("Sub-app {id} was not found"))
}

pub struct NewSubApp<'a> {
    pub executable: &'a str,
    pub app_name: Option<&'a str>,
    pub label: &'a str,
    pub icon: Option<&'a str>,
    pub title_pattern: &'a str,
    pub match_mode: TitleMatchMode,
}

fn normalize_icon(icon: Option<&str>) -> Option<String> {
    icon.map(str::trim)
        .filter(|icon| !icon.is_empty() && icon.len() <= 40)
        .map(str::to_string)
}

/// Adds a sub-app to the list, unassigned.
pub fn create_sub_app(db: &Db, new: NewSubApp<'_>) -> Result<ContextSubApp> {
    let executable = require_nonempty_trimmed("App", new.executable)?.to_lowercase();
    let label = normalize_label(new.label)?;
    let pattern = normalize_pattern(new.title_pattern)?;
    let conn = lock_conn(db)?;
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM context_sub_apps
              WHERE executable = ?1 AND title_pattern = ?2 COLLATE NOCASE AND match_mode = ?3",
            params![executable, pattern, new.match_mode.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if existing.is_some() {
        anyhow::bail!("A sub-app with this app and title rule already exists");
    }
    conn.execute(
        "INSERT INTO context_sub_apps
           (uuid, context_id, executable, app_name, label, icon, title_pattern, match_mode, platform)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            uuid::Uuid::new_v4().to_string(),
            executable,
            new.app_name.map(str::trim).filter(|v| !v.is_empty()),
            label,
            normalize_icon(new.icon),
            pattern,
            new.match_mode.as_str(),
            current_platform_tag(),
        ],
    )?;
    query_sub_app_conn(&conn, conn.last_insert_rowid())
}

/// Puts a sub-app in a Context, or back in the list with `None`. A sub-app
/// belongs to at most one Context, so assigning moves it.
pub fn assign_sub_app(db: &Db, id: i64, context_id: Option<i64>) -> Result<ContextSubApp> {
    let conn = lock_conn(db)?;
    if let Some(context_id) = context_id {
        if query_context_conn(&conn, context_id)?.is_everywhere {
            anyhow::bail!("The Everywhere context cannot have sub-apps");
        }
    }
    let changed = conn.execute(
        "UPDATE context_sub_apps SET context_id = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![context_id, id],
    )?;
    require_row_changed(changed, "Sub-app", id)?;
    query_sub_app_conn(&conn, id)
}

pub fn delete_sub_app(db: &Db, id: i64) -> Result<()> {
    let conn = lock_conn(db)?;
    let changed = conn.execute("DELETE FROM context_sub_apps WHERE id = ?1", params![id])?;
    require_row_changed(changed, "Sub-app", id)
}

/// The most specific sub-app rule matching this app and window title:
/// `equals` over `starts_with` over `contains`, then the longest pattern.
pub(crate) fn resolve_sub_app_conn(
    conn: &rusqlite::Connection,
    executable: &str,
    window_title: &str,
) -> Result<Option<ContextSubApp>> {
    if window_title.trim().is_empty() {
        return Ok(None);
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT {SUB_APP_COLUMNS} FROM context_sub_apps
          WHERE executable = ?1 AND context_id IS NOT NULL"
    ))?;
    let candidates = stmt
        .query_map(params![executable.trim().to_lowercase()], sub_app_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(candidates
        .into_iter()
        .filter(|sub| title_rule_matches(&sub.title_pattern, sub.match_mode, window_title))
        .max_by_key(|sub| (sub.match_mode.specificity(), normalize_title(&sub.title_pattern).len())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_match_case_and_whitespace_insensitively() {
        assert!(title_rule_matches("verenu  discord", TitleMatchMode::Contains, "#general - Verenu Discord"));
        assert!(title_rule_matches("#general", TitleMatchMode::StartsWith, "#General   - Verenu"));
        assert!(!title_rule_matches("#general", TitleMatchMode::Equals, "#general - Verenu"));
        assert!(!title_rule_matches("  ", TitleMatchMode::Contains, "anything"));
    }

    #[test]
    fn proposal_strips_counters_markers_and_app_suffix() {
        assert_eq!(
            propose_title_pattern("(3) #design | Acme - Slack", Some("Slack")),
            "#design | Acme"
        );
        assert_eq!(
            propose_title_pattern("● main.rs - verenu - Visual Studio Code", Some("Visual Studio Code")),
            "main.rs - verenu"
        );
        assert_eq!(propose_title_pattern("[12] Inbox", None), "Inbox");
        assert_eq!(propose_title_pattern("notes.md*", None), "notes.md");
        // A bracketed word is content, not an unread counter.
        assert_eq!(propose_title_pattern("[WIP] Plan", None), "[WIP] Plan");
    }

    fn add(db: &Db, executable: &str, label: &str, pattern: &str, mode: TitleMatchMode) -> ContextSubApp {
        create_sub_app(db, NewSubApp {
            executable,
            app_name: None,
            label,
            icon: None,
            title_pattern: pattern,
            match_mode: mode,
        })
        .unwrap()
    }

    #[test]
    fn most_specific_assigned_sub_app_wins_and_unassigned_ones_are_ignored() {
        let db = open(":memory:").expect("db");
        let work = insert_context_returning(&db, "Work", None, None, None, None, false).unwrap();
        let design = insert_context_returning(&db, "Design", None, None, None, None, false).unwrap();
        let broad = add(&db, "Discord", "Acme", "Acme", TitleMatchMode::Contains);
        let narrow = add(&db, "discord", "Acme design", "#design | Acme", TitleMatchMode::StartsWith);
        assert!(broad.context_id.is_none(), "new sub-apps start in the list");

        {
            let conn = lock_conn(&db).unwrap();
            assert!(resolve_sub_app_conn(&conn, "discord", "#design | Acme").unwrap().is_none());
        }
        assign_sub_app(&db, broad.id, Some(work.id)).unwrap();
        assign_sub_app(&db, narrow.id, Some(design.id)).unwrap();

        let conn = lock_conn(&db).unwrap();
        let hit = resolve_sub_app_conn(&conn, "DISCORD", "#design | Acme - Discord").unwrap().unwrap();
        assert_eq!(hit.context_id, Some(design.id));
        let hit = resolve_sub_app_conn(&conn, "discord", "#random | Acme - Discord").unwrap().unwrap();
        assert_eq!(hit.context_id, Some(work.id));
        drop(conn);

        let everywhere = query_contexts(&db).unwrap().into_iter().find(|c| c.is_everywhere).unwrap();
        assert!(assign_sub_app(&db, broad.id, Some(everywhere.id)).is_err());
    }

    #[test]
    fn deleting_a_context_returns_its_sub_apps_to_the_list() {
        let db = open(":memory:").expect("db");
        let work = insert_context_returning(&db, "Work", None, None, None, None, false).unwrap();
        let sub = add(&db, "slack", "Acme", "Acme", TitleMatchMode::Contains);
        assign_sub_app(&db, sub.id, Some(work.id)).unwrap();
        delete_context(&db, work.id).unwrap();
        let listed = query_sub_apps(&db).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].context_id, None);
    }
}
