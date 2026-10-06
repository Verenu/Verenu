use super::*;
use crate::api::github::CommitSnapshot;

const CACHE_SECONDS: i64 = 15 * 60;
const MAX_TZ_OVERRIDE_BYTES: usize = 4096;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
struct CachedCommitSnapshot {
    #[serde(flatten)]
    snapshot: CommitSnapshot,
    // Optional for caches written before timezone identity was recorded.
    #[serde(default)]
    timezone_id: Option<String>,
}

fn timezone_id_for_sources(
    tz_override: Option<&[u8]>,
    system_timezone: Option<&str>,
) -> Option<String> {
    if let Some(tz_override) = tz_override {
        if tz_override.len() > MAX_TZ_OVERRIDE_BYTES {
            return None;
        }
        use sha2::Digest;
        let digest = sha2::Sha256::digest(tz_override);
        let mut identity = String::from("unix-tz-sha256:");
        use std::fmt::Write;
        for byte in digest {
            let _ = write!(identity, "{byte:02x}");
        }
        return Some(identity);
    }
    system_timezone
        .map(str::trim)
        .filter(|timezone| !timezone.is_empty())
        .map(|timezone| format!("iana:{timezone}"))
}

fn current_timezone_id() -> Option<String> {
    // Chrono honors Unix TZ overrides, while iana-time-zone reports the system
    // zone. Hash a bounded override value so equal current offsets with different
    // historical rules cannot share cache identity. Non-Unicode values are
    // ignored here because Chrono ignores them too.
    #[cfg(unix)]
    let tz_override = std::env::var("TZ").ok();
    #[cfg(not(unix))]
    let tz_override: Option<String> = None;
    let system_timezone = iana_time_zone::get_timezone().ok();
    timezone_id_for_sources(
        tz_override.as_deref().map(str::as_bytes),
        system_timezone.as_deref(),
    )
}

fn cached_snapshot_for_user(
    value: Option<serde_json::Value>,
    username: &str,
) -> Option<CachedCommitSnapshot> {
    value
        .and_then(|value| serde_json::from_value::<CachedCommitSnapshot>(value).ok())
        .filter(|cache| cache.snapshot.username.eq_ignore_ascii_case(username))
}

fn cache_is_fresh(
    cache: &CachedCommitSnapshot,
    username: &str,
    today: chrono::NaiveDate,
    current_offset: i32,
    current_timezone_id: Option<&str>,
    now: i64,
) -> bool {
    let timezone_matches = cache
        .timezone_id
        .as_deref()
        .zip(current_timezone_id)
        .is_some_and(|(cached, current)| cached == current);
    timezone_matches
        && cache.snapshot.matches(username, today, current_offset)
        && (0..CACHE_SECONDS).contains(&(now - cache.snapshot.fetched_at))
}

fn cached_failure_snapshot(
    mut cache: CachedCommitSnapshot,
    error: &str,
    current_offset: i32,
    current_timezone_id: Option<&str>,
) -> CommitSnapshot {
    let timezone_note = match (cache.timezone_id.as_deref(), current_timezone_id) {
        (Some(cached), Some(current)) if cached != current => {
            " The time zone changed since this fetch; the original daily buckets are retained."
        }
        (None, _) | (_, None) => {
            " The time zone identity is missing or unavailable; the original daily buckets are retained."
        }
        _ if cache.snapshot.utc_offset != current_offset => {
            " The local UTC offset changed since this fetch; the original daily buckets are retained."
        }
        _ => "",
    };
    let partial = if cache.snapshot.complete {
        ""
    } else {
        " Counts are lower bounds; days without results are unknown."
    };
    cache.snapshot.warning = Some(format!(
        "Showing cached counts. {error}{timezone_note}{partial}"
    ));
    cache.snapshot
}

#[derive(serde::Serialize)]
pub struct GithubUsernameSuggestion {
    username: String,
    source: &'static str,
}

fn parse_username_output(output: &[u8]) -> Option<String> {
    let username = std::str::from_utf8(output).ok()?.trim();
    crate::api::github::valid_username(username).then(|| username.to_owned())
}

#[cfg(not(target_os = "android"))]
fn command_username(program: &str, args: &[&str], seconds: u64) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir(std::env::temp_dir())
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GH_DEBUG");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.take(256).read_to_end(&mut bytes);
        let _ = sender.send(bytes);
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                return parse_username_output(
                    &receiver.recv_timeout(Duration::from_millis(100)).ok()?,
                );
            }
            Ok(None) if start.elapsed() < Duration::from_secs(seconds) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[tauri::command]
pub async fn get_github_username_suggestion() -> Result<Option<GithubUsernameSuggestion>, String> {
    run_blocking("github_username_suggestion", || {
        #[cfg(not(target_os = "android"))]
        {
            // Ask the CLI for just the login. Never read or return its token,
            // name, email, credential-helper output, or browser cookies.
            let args = ["api", "--hostname", "github.com", "user", "--jq", ".login"];
            let username = command_username("gh", &args, 8);
            #[cfg(target_os = "macos")]
            let username = username.or_else(|| {
                ["/opt/homebrew/bin/gh", "/usr/local/bin/gh"]
                    .iter()
                    .filter(|path| std::path::Path::new(path).is_file())
                    .find_map(|path| command_username(path, &args, 8))
            });
            if let Some(username) = username {
                return Ok(Some(GithubUsernameSuggestion {
                    username,
                    source: "github_cli",
                }));
            }
            if let Some(username) =
                command_username("git", &["config", "--global", "--get", "github.user"], 2)
            {
                return Ok(Some(GithubUsernameSuggestion {
                    username,
                    source: "git_config",
                }));
            }
        }
        Ok(None)
    })
    .await
}

#[cfg(test)]
mod suggestion_tests {
    use super::*;
    #[cfg(all(unix, not(target_os = "android")))]
    #[test]
    fn cli_detection_is_bounded_and_tolerates_missing_or_invalid_tools() {
        assert_eq!(
            command_username("/bin/sh", &["-c", "printf 'fixture-user\\n'"], 1),
            Some("fixture-user".to_owned())
        );
        assert!(command_username("/bin/sh", &["-c", "printf 'not a username'"], 1).is_none());
        assert!(command_username("/bin/sh", &["-c", "while :; do :; done"], 0).is_none());
        assert!(command_username("verenu-fixture-missing-cli", &[], 1).is_none());
    }

    #[test]
    fn suggestion_accepts_only_a_single_valid_login() {
        assert_eq!(
            parse_username_output(b"fixture-user\n"),
            Some("fixture-user".to_owned())
        );
        for output in [
            b"".as_slice(),
            b"A Person",
            b"person@example.com",
            b"user\nother",
            b"--bad",
            b"ghp_secret/invalid",
        ] {
            assert!(parse_username_output(output).is_none());
        }
    }
}
// Serialize refreshes across remounts and simultaneous IPC callers.
static REFRESH: tokio::sync::Mutex<(String, i64)> =
    tokio::sync::Mutex::const_new((String::new(), 0));

#[tauri::command]
pub async fn get_github_commits(
    app: AppHandle,
    refresh: Option<bool>,
) -> Result<Option<CommitSnapshot>, String> {
    let mut attempt = REFRESH.lock().await;
    let settings = store::settings_handle(&app)?;
    let username = settings
        .get(store::GITHUB_USERNAME)
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    if username.is_empty() {
        return Ok(None);
    }
    if !crate::api::github::valid_username(&username) {
        return Err("Enter a valid GitHub username.".to_owned());
    }
    let local_now = chrono::Local::now();
    let today = local_now.date_naive();
    let current_offset = local_now.offset().local_minus_utc();
    let timezone_id = current_timezone_id();
    let cached = cached_snapshot_for_user(settings.get(store::GITHUB_COMMIT_CACHE), &username);
    let now = chrono::Utc::now().timestamp();
    if !refresh.unwrap_or(false) {
        if let Some(cache) = cached.as_ref().filter(|cache| {
            cache_is_fresh(
                cache,
                &username,
                today,
                current_offset,
                timezone_id.as_deref(),
                now,
            )
        }) {
            return Ok(Some(cache.snapshot.clone()));
        }
    }
    let timezone_before_fetch = current_timezone_id();
    let result = if attempt.0.eq_ignore_ascii_case(&username) && now - attempt.1 < 60 {
        Err("Please wait a minute before refreshing GitHub again.".to_owned())
    } else {
        *attempt = (username.clone(), now);
        match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            crate::api::github::fetch_commits(&username),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err("GitHub took too long to respond. Try again later.".to_owned()),
        }
    };
    match result {
        Ok(snapshot) => {
            // A disconnect or username change during the request must not restore its cache.
            let timezone_after_fetch = current_timezone_id();
            let timezone_id = timezone_before_fetch
                .as_deref()
                .zip(timezone_after_fetch.as_deref())
                .filter(|(before, after)| before == after)
                .map(|(timezone, _)| timezone.to_owned());
            let cache = CachedCommitSnapshot {
                snapshot: snapshot.clone(),
                timezone_id,
            };
            let value =
                serde_json::to_value(cache).map_err(|_| "Could not cache GitHub counts.")?;
            run_blocking("cache_github_commits", move || {
                settings
                    .save_value_if_owner_matches(
                        store::GITHUB_USERNAME,
                        &serde_json::json!(username),
                        store::GITHUB_COMMIT_CACHE,
                        value,
                    )
                    .map(|_| ())
            })
            .await?;
            Ok(Some(snapshot))
        }
        Err(error) => match cached {
            Some(cache) => {
                let current_offset = chrono::Local::now().offset().local_minus_utc();
                let current_timezone_id = current_timezone_id();
                Ok(Some(cached_failure_snapshot(
                    cache,
                    &error,
                    current_offset,
                    current_timezone_id.as_deref(),
                )))
            }
            None => Err(error),
        },
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn cache(username: &str, offset: i32, timezone_id: Option<&str>) -> CachedCommitSnapshot {
        CachedCommitSnapshot {
            snapshot: snapshot(username, offset),
            timezone_id: timezone_id.map(str::to_owned),
        }
    }

    fn snapshot(username: &str, offset: i32) -> CommitSnapshot {
        let today = chrono::Local::now().date_naive();
        CommitSnapshot {
            username: username.to_owned(),
            fetched_at: 100,
            start_day: (today - chrono::Duration::days(crate::api::github::HISTORY_DAYS - 1))
                .to_string(),
            end_day: today.to_string(),
            utc_offset: offset,
            complete: true,
            daily: vec![crate::api::github::CommitDay {
                day: today.to_string(),
                commits: 4,
            }],
            warning: None,
        }
    }

    #[test]
    fn offset_change_forces_refresh_and_failed_refresh_keeps_cached_buckets() {
        let cached = cache("octocat", -28_800, Some("America/Los_Angeles"));
        let today = chrono::Local::now().date_naive();
        assert!(cache_is_fresh(
            &cached,
            "octocat",
            today,
            -28_800,
            Some("America/Los_Angeles"),
            200
        ));
        assert!(!cache_is_fresh(
            &cached,
            "octocat",
            today,
            -25_200,
            Some("America/Los_Angeles"),
            200
        ));

        let stale = cached_failure_snapshot(
            cached.clone(),
            "GitHub is unavailable.",
            -25_200,
            Some("America/Los_Angeles"),
        );
        assert_eq!(stale.utc_offset, cached.snapshot.utc_offset);
        assert_eq!(stale.daily[0].day, cached.snapshot.daily[0].day);
        assert_eq!(stale.daily[0].commits, 4);
        let warning = stale.warning.unwrap();
        assert!(warning.contains("Showing cached counts"));
        assert!(warning.contains("UTC offset changed"));
        assert!(warning.contains("original daily buckets are retained"));
    }

    #[test]
    fn same_current_offset_with_different_timezone_rules_forces_refresh_and_warns_on_fallback() {
        // New York and Lima can both be UTC-5, but New York observes DST.
        let new_york = timezone_id_for_sources(Some(b"America/New_York"), None).unwrap();
        let lima = timezone_id_for_sources(Some(b"America/Lima"), None).unwrap();
        let cached = cache("octocat", -18_000, Some(&new_york));
        let today = chrono::Local::now().date_naive();
        assert!(!cache_is_fresh(
            &cached,
            "octocat",
            today,
            -18_000,
            Some(&lima),
            200
        ));

        let stale = cached_failure_snapshot(
            cached.clone(),
            "GitHub is unavailable.",
            -18_000,
            Some(&lima),
        );
        assert_eq!(stale.utc_offset, -18_000);
        assert_eq!(stale.daily[0].day, cached.snapshot.daily[0].day);
        assert_eq!(stale.daily[0].commits, 4);
        let warning = stale.warning.unwrap();
        assert!(warning.contains("time zone changed"));
        assert!(warning.contains("original daily buckets are retained"));
    }

    #[test]
    fn timezone_identity_distinguishes_overrides_system_zone_and_unknown_values() {
        let new_york = timezone_id_for_sources(Some(b"America/New_York"), Some("Etc/UTC"));
        let lima = timezone_id_for_sources(Some(b"America/Lima"), Some("Etc/UTC"));
        let posix = timezone_id_for_sources(Some(b"EST5EDT,M3.2.0/2,M11.1.0/2"), None);
        let empty_override = timezone_id_for_sources(Some(b""), Some("Etc/UTC"));
        let system = timezone_id_for_sources(None, Some("America/New_York"));

        assert_ne!(new_york, lima);
        assert_ne!(new_york, system);
        assert!(posix.is_some());
        assert!(empty_override.is_some());
        assert_ne!(empty_override, system);
        assert_eq!(
            timezone_id_for_sources(
                Some(&vec![b'x'; MAX_TZ_OVERRIDE_BYTES + 1]),
                Some("Etc/UTC")
            ),
            None
        );
        assert_eq!(timezone_id_for_sources(None, None), None);
    }

    #[test]
    fn legacy_cache_without_timezone_identity_is_stale_but_remains_fallback_data() {
        let old_value = serde_json::to_value(snapshot("octocat", 0)).unwrap();
        let legacy = cached_snapshot_for_user(Some(old_value.clone()), "octocat").unwrap();
        assert!(legacy.timezone_id.is_none());

        let today = chrono::Local::now().date_naive();
        assert!(!cache_is_fresh(
            &legacy,
            "octocat",
            today,
            0,
            Some("America/Los_Angeles"),
            200
        ));
        assert!(!cache_is_fresh(&legacy, "octocat", today, 0, None, 200));

        let stale = cached_failure_snapshot(
            legacy,
            "GitHub is unavailable.",
            0,
            Some("America/Los_Angeles"),
        );
        assert_eq!(stale.daily[0].commits, 4);
        let warning = stale.warning.unwrap();
        assert!(warning.contains("identity is missing or unavailable"));
        assert!(warning.contains("original daily buckets are retained"));
    }

    #[test]
    fn unknown_current_timezone_is_never_fresh_and_warns_on_fallback() {
        let cached = cache("octocat", 0, Some("Etc/UTC"));
        let today = chrono::Local::now().date_naive();
        assert!(!cache_is_fresh(&cached, "octocat", today, 0, None, 200));
        let stale = cached_failure_snapshot(cached, "GitHub is unavailable.", 0, None);
        assert!(stale
            .warning
            .unwrap()
            .contains("identity is missing or unavailable"));
    }

    #[test]
    fn cached_snapshot_is_only_reused_for_the_matching_account() {
        let value = serde_json::to_value(cache("octocat", 0, Some("Etc/UTC"))).unwrap();
        assert!(cached_snapshot_for_user(Some(value.clone()), "OCTOCAT").is_some());
        assert!(cached_snapshot_for_user(Some(value), "other-user").is_none());
    }
}
