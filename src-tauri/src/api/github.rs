//! Optional GitHub activity counts. The public profile calendar is primary;
//! unauthenticated public commit search is the fallback. No credentials,
//! commit messages, or repository names are retained or returned to the UI.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const HISTORY_DAYS: i64 = 90;
const MAX_PAGES: usize = 10;
const MAX_PROFILE_BYTES: usize = 8 * 1024 * 1024;

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
    #[serde(default)]
    pub commits: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivitySource {
    Contributions,
    #[default]
    PublicCommits,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommitSnapshot {
    /// Missing on old caches; those were produced by the public search API.
    #[serde(default)]
    pub source: ActivitySource,
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
    let now = Local::now();
    let today = now.date_naive();
    let start = today - Duration::days(HISTORY_DAYS - 1);

    let profile = fetch_contributions(username, start, today, now.offset().local_minus_utc()).await;
    fetch_public_if_unavailable(profile, || fetch_public_commits(username)).await
}

async fn fetch_public_if_unavailable<F, Fut>(
    profile: Result<CommitSnapshot, String>,
    fetch_public: F,
) -> Result<CommitSnapshot, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CommitSnapshot, String>>,
{
    match profile {
        Ok(snapshot) => Ok(snapshot),
        Err(_) => fetch_public().await.map(mark_public_fallback),
    }
}

fn mark_public_fallback(mut snapshot: CommitSnapshot) -> CommitSnapshot {
    let fallback_note = "GitHub's contribution calendar could not be read. Showing public commits returned by GitHub's search; it can miss other contributions and is limited by GitHub's index and result cap.";
    snapshot.warning = Some(match snapshot.warning {
        Some(warning) => format!("{fallback_note} {warning}"),
        None => fallback_note.to_owned(),
    });
    snapshot
}

async fn fetch_profile_html(
    username: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<String, String> {
    let mut url = reqwest::Url::parse(&format!(
        "https://github.com/users/{username}/contributions"
    ))
    .map_err(|_| "GitHub's contribution calendar is unavailable.".to_owned())?;
    url.query_pairs_mut()
        .append_pair("from", &start.to_string())
        .append_pair("to", &end.to_string());
    let mut response = super::client::hardened()
        .get(url)
        .header("User-Agent", "Verenu")
        .header("Accept", "text/html")
        .header("Accept-Language", "en-US,en;q=0.9")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| "Could not reach GitHub's contribution calendar.".to_owned())?;
    if !response.status().is_success() {
        return Err(status_error(response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROFILE_BYTES as u64)
    {
        return Err("GitHub's contribution calendar response was too large.".to_owned());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "GitHub returned an unreadable contribution calendar.".to_owned())?
    {
        if body.len().saturating_add(chunk.len()) > MAX_PROFILE_BYTES {
            return Err("GitHub's contribution calendar response was too large.".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body)
        .map_err(|_| "GitHub returned an unreadable contribution calendar.".to_owned())
}

fn tooltip_day_matches(text: &str, day: NaiveDate) -> bool {
    let month = day.format("%B").to_string();
    let mut parts = text.trim().trim_end_matches('.').split_whitespace();
    if parts.next() != Some(month.as_str()) {
        return false;
    }
    let Some(day_token) = parts.next() else {
        return false;
    };
    let day_number = day_token
        .trim_end_matches([',', '.'])
        .trim_end_matches("st")
        .trim_end_matches("nd")
        .trim_end_matches("rd")
        .trim_end_matches("th")
        .parse::<u32>();
    if day_number.ok() != Some(day.day()) {
        return false;
    }
    match (parts.next(), parts.next()) {
        (None, None) => true,
        (Some(year), None) => year.trim_end_matches(',').parse::<i32>().ok() == Some(day.year()),
        _ => false,
    }
}

fn parse_tooltip_count(text: &str, day: NaiveDate) -> Option<u64> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(date) = text.strip_prefix("No contributions on ") {
        return tooltip_day_matches(date, day).then_some(0);
    }
    let mut parts = text.splitn(4, ' ');
    let count = parse_count(parts.next()?)?;
    let noun = parts.next()?;
    if (count == 1 && noun != "contribution") || (count != 1 && noun != "contributions") {
        return None;
    }
    if parts.next()? != "on" || !tooltip_day_matches(parts.next()?, day) {
        return None;
    }
    Some(count)
}

fn parse_count(value: &str) -> Option<u64> {
    if !value.contains(',') {
        return value.parse::<u64>().ok();
    }
    let mut groups = value.split(',');
    let first = groups.next()?;
    if first.is_empty() || first.len() > 3 || !first.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut count = first.parse::<u64>().ok()?;
    for group in groups {
        if group.len() != 3 || !group.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        count = count
            .checked_mul(1_000)?
            .checked_add(group.parse::<u64>().ok()?)?;
    }
    Some(count)
}

/// Reads the exact counts shown by GitHub's public profile contribution
/// calendar. The calendar can include anonymized private contributions when
/// its owner enables that profile setting; no GitHub credentials are used.
fn parse_contribution_calendar(
    html: &str,
    username: &str,
    start: NaiveDate,
    end: NaiveDate,
    fetched_at: i64,
    utc_offset: i32,
) -> Result<CommitSnapshot, String> {
    use scraper::{Html, Selector};

    let day_selector = Selector::parse("td.ContributionCalendar-day[data-date][data-level][id]")
        .expect("valid contribution cell selector");
    let tooltip_selector = Selector::parse("tool-tip[for]").expect("valid tooltip selector");
    let document = Html::parse_document(html);
    let mut cells = BTreeMap::<NaiveDate, (String, u8)>::new();
    let mut ids = HashSet::new();
    for cell in document.select(&day_selector) {
        let id = cell.value().attr("id").unwrap_or_default();
        let date_text = cell.value().attr("data-date").unwrap_or_default();
        let level_text = cell.value().attr("data-level").unwrap_or_default();
        let day = NaiveDate::parse_from_str(date_text, "%Y-%m-%d")
            .map_err(|_| "GitHub's contribution calendar has an invalid date.".to_owned())?;
        let level = level_text
            .parse::<u8>()
            .ok()
            .filter(|level| *level <= 4)
            .ok_or_else(|| {
                "GitHub's contribution calendar has an invalid activity level.".to_owned()
            })?;
        if id.is_empty()
            || !ids.insert(id.to_owned())
            || cells.insert(day, (id.to_owned(), level)).is_some()
        {
            return Err("GitHub's contribution calendar has duplicate days.".to_owned());
        }
    }
    if cells.is_empty() {
        return Err("GitHub's contribution calendar markup is unsupported.".to_owned());
    }

    let mut tooltips = BTreeMap::<String, String>::new();
    for tooltip in document.select(&tooltip_selector) {
        let id = tooltip.value().attr("for").unwrap_or_default();
        if ids.contains(id) {
            let text = tooltip.text().collect::<String>();
            if tooltips.insert(id.to_owned(), text).is_some() {
                return Err("GitHub's contribution calendar has duplicate day details.".to_owned());
            }
        }
    }

    let mut counts = BTreeMap::new();
    for (day, (id, _level)) in &cells {
        let tooltip = tooltips
            .get(id)
            .ok_or_else(|| "GitHub's contribution calendar is missing day details.".to_owned())?;
        let count = parse_tooltip_count(tooltip, *day).ok_or_else(|| {
            "GitHub's contribution calendar has unsupported day details.".to_owned()
        })?;
        counts.insert(*day, count);
    }

    let expected_days = (end - start).num_days() + 1;
    if expected_days <= 0
        || (0..expected_days).any(|offset| !counts.contains_key(&(start + Duration::days(offset))))
    {
        return Err(
            "GitHub's contribution calendar does not cover the requested dates.".to_owned(),
        );
    }
    Ok(CommitSnapshot {
        source: ActivitySource::Contributions,
        username: username.to_ascii_lowercase(),
        fetched_at,
        start_day: start.to_string(),
        end_day: end.to_string(),
        utc_offset,
        complete: true,
        daily: (0..expected_days)
            .map(|offset| {
                let day = start + Duration::days(offset);
                CommitDay {
                    day: day.to_string(),
                    commits: counts.get(&day).copied(),
                }
            })
            .collect(),
        warning: None,
    })
}

async fn fetch_contributions(
    username: &str,
    start: NaiveDate,
    end: NaiveDate,
    utc_offset: i32,
) -> Result<CommitSnapshot, String> {
    let fetched_at = chrono::Utc::now().timestamp();
    let mut daily = Vec::new();
    for (segment_start, segment_end) in calendar_segments(start, end) {
        let html = fetch_profile_html(username, segment_start, segment_end).await?;
        let segment = parse_contribution_calendar(
            &html,
            username,
            segment_start,
            segment_end,
            fetched_at,
            utc_offset,
        )?;
        daily.extend(segment.daily);
    }
    if daily.len() as i64 != (end - start).num_days() + 1 {
        return Err(
            "GitHub's contribution calendar does not cover the requested dates.".to_owned(),
        );
    }
    Ok(CommitSnapshot {
        source: ActivitySource::Contributions,
        username: username.to_ascii_lowercase(),
        fetched_at,
        start_day: start.to_string(),
        end_day: end.to_string(),
        utc_offset,
        complete: true,
        daily,
        warning: None,
    })
}

fn calendar_segments(start: NaiveDate, end: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
    let mut segments = Vec::new();
    let mut segment_start = start;
    while segment_start <= end {
        let year_end =
            NaiveDate::from_ymd_opt(segment_start.year(), 12, 31).expect("valid calendar year end");
        let segment_end = year_end.min(end);
        segments.push((segment_start, segment_end));
        let Some(next) = segment_end.succ_opt() else {
            break;
        };
        segment_start = next;
    }
    segments
}

async fn fetch_public_commits(username: &str) -> Result<CommitSnapshot, String> {
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
        source: ActivitySource::PublicCommits,
        username: username.to_ascii_lowercase(),
        fetched_at: chrono::Utc::now().timestamp(),
        start_day: start.to_string(),
        end_day: today.to_string(),
        utc_offset: now.offset().local_minus_utc(),
        complete,
        daily: counts
            .into_iter()
            .map(|(day, commits)| CommitDay {
                day,
                // With incomplete search results, a returned commit is a
                // useful lower bound. A day with no indexed result is unknown,
                // not a confirmed zero.
                commits: (complete || commits > 0).then_some(commits),
            })
            .collect(),
        warning: (!complete).then(|| "GitHub's public commit search returned partial results. Values may be incomplete; days without indexed results are unknown.".to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar_fixture() -> String {
        r#"
        <table class="ContributionCalendar-grid">
          <tbody>
            <tr>
              <td id="fixture-day-one" class="ContributionCalendar-day" data-date="2026-10-01" data-level="0" role="gridcell"></td>
              <td id="fixture-day-two" class="ContributionCalendar-day" data-date="2026-10-02" data-level="3" role="gridcell"></td>
              <td id="fixture-day-three" class="ContributionCalendar-day" data-date="2026-10-03" data-level="1" role="gridcell"></td>
            </tr>
          </tbody>
        </table>
        <tool-tip for="fixture-day-one">No contributions on October 1st.</tool-tip>
        <tool-tip for="fixture-day-two">4 contributions on October 2nd.</tool-tip>
        <tool-tip for="fixture-day-three">1 contribution on October 3rd.</tool-tip>
        "#
        .to_owned()
    }

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

    #[test]
    fn profile_calendar_uses_tooltip_counts_and_filters_full_year_markup() {
        let snapshot = parse_contribution_calendar(
            &calendar_fixture(),
            "fixture-user",
            NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 3).unwrap(),
            123,
            0,
        )
        .unwrap();
        assert_eq!(snapshot.source, ActivitySource::Contributions);
        assert_eq!(snapshot.start_day, "2026-10-02");
        assert_eq!(snapshot.end_day, "2026-10-03");
        assert_eq!(snapshot.daily[0].commits, Some(4));
        assert_eq!(snapshot.daily[1].commits, Some(1));
        assert!(snapshot.complete);
    }

    #[test]
    fn profile_calendar_rejects_malformed_and_partial_day_markup() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let cases = [
            calendar_fixture().replace("data-date=\"2026-10-01\"", "data-date=\"bad-date\""),
            calendar_fixture().replace("data-level=\"3\"", "data-level=\"9\""),
            calendar_fixture().replace(
                "4 contributions on October 2nd.",
                "many contributions on October 2nd.",
            ),
            calendar_fixture().replace(" for=\"fixture-day-three\"", " for=\"other-day\""),
            calendar_fixture().replace("2026-10-03", "2026-10-04"),
        ];
        for html in cases {
            assert!(
                parse_contribution_calendar(&html, "fixture-user", start, end, 123, 0).is_err(),
                "malformed or incomplete calendar markup must fail over"
            );
        }
    }

    #[test]
    fn public_search_fallback_keeps_its_source_and_explains_its_limit() {
        let snapshot = CommitSnapshot {
            source: ActivitySource::PublicCommits,
            username: "fixture-user".to_owned(),
            fetched_at: 123,
            start_day: "2026-10-01".to_owned(),
            end_day: "2026-10-02".to_owned(),
            utc_offset: 0,
            complete: true,
            daily: vec![],
            warning: None,
        };
        let fallback = mark_public_fallback(snapshot);
        assert_eq!(fallback.source, ActivitySource::PublicCommits);
        let warning = fallback.warning.unwrap();
        assert!(warning.contains("public commits"));
        assert!(warning.contains("index and result cap"));
    }

    #[tokio::test]
    async fn successful_profile_data_skips_public_search() {
        let profile = minimal_snapshot(ActivitySource::Contributions);
        let searched = std::cell::Cell::new(false);
        let result = fetch_public_if_unavailable(Ok(profile), || {
            searched.set(true);
            std::future::ready(Err("should not run".to_owned()))
        })
        .await
        .unwrap();
        assert_eq!(result.source, ActivitySource::Contributions);
        assert!(!searched.get());
    }

    #[tokio::test]
    async fn unsupported_profile_data_uses_labeled_public_search() {
        let public = minimal_snapshot(ActivitySource::PublicCommits);
        let result = fetch_public_if_unavailable(Err("unsupported markup".to_owned()), || {
            std::future::ready(Ok(public))
        })
        .await
        .unwrap();
        assert_eq!(result.source, ActivitySource::PublicCommits);
        assert!(result.warning.unwrap().contains("public commits"));
    }

    fn minimal_snapshot(source: ActivitySource) -> CommitSnapshot {
        CommitSnapshot {
            source,
            username: "fixture-user".to_owned(),
            fetched_at: 123,
            start_day: "2026-10-01".to_owned(),
            end_day: "2026-10-02".to_owned(),
            utc_offset: 0,
            complete: true,
            daily: vec![],
            warning: None,
        }
    }

    #[test]
    fn old_unmarked_snapshots_deserialize_as_public_commits() {
        let snapshot: CommitSnapshot = serde_json::from_value(serde_json::json!({
            "username": "fixture-user", "fetched_at": 123,
            "start_day": "2026-10-01", "end_day": "2026-10-02",
            "utc_offset": 0, "complete": false,
            "daily": [{"day": "2026-10-01", "commits": 0}],
            "warning": null
        }))
        .unwrap();
        assert_eq!(snapshot.source, ActivitySource::PublicCommits);
        assert_eq!(snapshot.daily[0].commits, Some(0));
    }

    #[test]
    fn contribution_calendar_splits_ranges_at_calendar_years() {
        let segments = calendar_segments(
            NaiveDate::from_ymd_opt(2025, 12, 30).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
        );
        assert_eq!(
            segments,
            vec![
                (
                    NaiveDate::from_ymd_opt(2025, 12, 30).unwrap(),
                    NaiveDate::from_ymd_opt(2025, 12, 31).unwrap()
                ),
                (
                    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                    NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()
                ),
            ]
        );
    }

    #[test]
    fn contribution_tooltip_counts_support_valid_grouping_only() {
        assert_eq!(parse_count("1,234"), Some(1234));
        assert_eq!(parse_count("1234567"), Some(1234567));
        for malformed in ["12,34", "1,,000", ",100", "1,0000", "1,000,00"] {
            assert_eq!(parse_count(malformed), None, "accepted {malformed}");
        }
    }
}
