//! Optional public commit counts. No credentials, commit messages, or repository
//! names are retained or returned to the renderer.

use chrono::{DateTime, Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const HISTORY_DAYS: i64 = 90;
const MAX_PAGES: usize = 10;

pub fn valid_username(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 39
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommitDay {
    pub day: String,
    pub commits: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommitSnapshot {
    pub username: String,
    pub fetched_at: i64,
    pub start_day: String,
    pub end_day: String,
    pub utc_offset: i32,
    pub complete: bool,
    pub daily: Vec<CommitDay>,
    #[serde(default)]
    pub warning: Option<String>,
}

impl CommitSnapshot {
    pub fn matches(&self, username: &str, today: NaiveDate, offset: i32) -> bool {
        self.username.eq_ignore_ascii_case(username)
            && self.end_day == today.to_string()
            && self.start_day == (today - Duration::days(HISTORY_DAYS - 1)).to_string()
            && self.utc_offset == offset
    }
}

#[derive(Deserialize)]
struct SearchPage {
    total_count: usize,
    incomplete_results: bool,
    items: Vec<SearchCommit>,
}

#[derive(Deserialize)]
struct SearchCommit {
    sha: String,
    commit: CommitMetadata,
}

#[derive(Deserialize)]
struct CommitMetadata {
    author: CommitAuthor,
}

#[derive(Deserialize)]
struct CommitAuthor {
    date: String,
}

fn count_page(
    page: &SearchPage,
    seen: &mut HashSet<String>,
    counts: &mut BTreeMap<String, u64>,
) -> bool {
    let mut valid = true;
    for item in &page.items {
        let Ok(date) = DateTime::parse_from_rfc3339(&item.commit.author.date) else {
            valid = false;
            continue;
        };
        // A commit may appear in several public forks. Count its SHA once.
        if seen.insert(item.sha.clone()) {
            if let Some(count) =
                counts.get_mut(&date.with_timezone(&Local).date_naive().to_string())
            {
                *count += 1;
            }
        }
    }
    valid
}

fn status_error(status: u16) -> String {
    match status {
        403 | 429 => "GitHub's request limit was reached. Try again in a few minutes.",
        404 | 422 => "GitHub could not find that username. Check the spelling and try again.",
        _ => "GitHub is unavailable right now. Try again later.",
    }
    .to_owned()
}

async fn request(path: &str) -> Result<reqwest::Response, String> {
    let response = super::client::hardened()
        .get(format!("https://api.github.com/{path}"))
        .header("User-Agent", "Verenu")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| "Could not reach GitHub. Check your connection and try again.".to_owned())?;
    if !response.status().is_success() {
        return Err(status_error(response.status().as_u16()));
    }
    Ok(response)
}

pub async fn fetch_commits(username: &str) -> Result<CommitSnapshot, String> {
    if !valid_username(username) {
        return Err(
            "Enter a GitHub username using letters, numbers, and single hyphens.".to_owned(),
        );
    }
    // Search returns zero results for some nonexistent users, so validate first.
    request(&format!("users/{username}")).await?;
    let now = Local::now();
    let today = now.date_naive();
    let start = today - Duration::days(HISTORY_DAYS - 1);
    // Date search uses UTC. Pad both ends, then bucket/filter locally like Insights.
    let query = format!(
        "author:{username} author-date:{}..{}",
        start - Duration::days(1),
        today + Duration::days(1)
    );
    let mut counts = BTreeMap::new();
    for i in 0..HISTORY_DAYS {
        counts.insert((start + Duration::days(i)).to_string(), 0);
    }
    let mut seen = HashSet::new();
    let mut complete = true;
    let mut fetched = 0;
    for page_number in 1..=MAX_PAGES {
        let mut url = reqwest::Url::parse("https://api.github.com/search/commits").unwrap();
        url.query_pairs_mut()
            .append_pair("q", &query)
            .append_pair("per_page", "100")
            .append_pair("sort", "author-date")
            .append_pair("order", "desc")
            .append_pair("page", &page_number.to_string());
        let path = format!("search/commits?{}", url.query().unwrap());
        let page = match request(&path).await {
            Ok(response) => super::Wire {
                hardened: true,
                ..Default::default()
            }
            .json::<SearchPage>(response)
            .await
            .map_err(|_| "GitHub returned an unreadable response. Try again later.".to_owned()),
            Err(error) => Err(error),
        }?;
        let valid_dates = count_page(&page, &mut seen, &mut counts);
        complete &= !page.incomplete_results && valid_dates;
        fetched += page.items.len();
        if fetched >= page.total_count {
            break;
        }
        if page.items.len() < 100 || page_number == MAX_PAGES {
            complete = false;
            break;
        }
    }
    Ok(CommitSnapshot {
        username: username.to_ascii_lowercase(),
        fetched_at: chrono::Utc::now().timestamp(),
        start_day: start.to_string(),
        end_day: today.to_string(),
        utc_offset: now.offset().local_minus_utc(),
        complete,
        daily: counts.into_iter().map(|(day, commits)| CommitDay { day, commits }).collect(),
        warning: (!complete).then(|| "GitHub returned partial results. Counts are lower bounds; days without results are unknown.".to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_username_cannot_inject_queries_or_paths() {
        for value in ["octocat", "user-123", "A"] {
            assert!(valid_username(value));
        }
        for value in [
            "",
            "-a",
            "a-",
            "a--b",
            "a/b",
            "a repo:private",
            "a?x=1",
            "é",
        ] {
            assert!(!valid_username(value));
        }
        assert!(!valid_username(&"a".repeat(40)));
    }

    #[test]
    fn github_counts_deduplicate_forks_and_ignore_outside_dates() {
        let today = Local::now().date_naive().to_string();
        let date = Local::now().to_rfc3339();
        let page: SearchPage = serde_json::from_value(serde_json::json!({
            "total_count": 3, "incomplete_results": false,
            "items": [
                {"sha":"a", "commit":{"author":{"date":date}}},
                {"sha":"a", "commit":{"author":{"date":date}}},
                {"sha":"b", "commit":{"author":{"date":"2000-01-01T00:00:00Z"}}}
            ]
        }))
        .unwrap();
        let mut counts = BTreeMap::from([(today.clone(), 0)]);
        assert!(count_page(&page, &mut HashSet::new(), &mut counts));
        assert_eq!(counts[&today], 1);
    }

    #[test]
    fn github_bad_dates_mark_counts_incomplete() {
        let page: SearchPage = serde_json::from_value(serde_json::json!({
            "total_count": 1, "incomplete_results": false,
            "items": [{"sha":"a", "commit":{"author":{"date":"invalid"}}}]
        }))
        .unwrap();
        assert!(!count_page(
            &page,
            &mut HashSet::new(),
            &mut BTreeMap::new()
        ));
    }
}
