use super::*;
use crate::api::github::CommitSnapshot;

const CACHE_SECONDS: i64 = 15 * 60;

fn cached_snapshot_for_user(
    value: Option<serde_json::Value>,
    username: &str,
) -> Option<CommitSnapshot> {
    value
        .and_then(|value| serde_json::from_value::<CommitSnapshot>(value).ok())
        .filter(|snapshot| snapshot.username.eq_ignore_ascii_case(username))
}

fn cache_is_fresh(
    snapshot: &CommitSnapshot,
    username: &str,
    today: chrono::NaiveDate,
    current_offset: i32,
    now: i64,
) -> bool {
    snapshot.matches(username, today, current_offset)
        && (0..CACHE_SECONDS).contains(&(now - snapshot.fetched_at))
}

fn cached_failure_snapshot(
    mut snapshot: CommitSnapshot,
    error: &str,
    current_offset: i32,
) -> CommitSnapshot {
    let offset_note = if snapshot.utc_offset != current_offset {
        " The local UTC offset changed since this fetch; the original daily buckets are retained."
    } else {
        ""
    };
    let partial = if snapshot.complete {
        ""
    } else {
        " Counts are lower bounds; days without results are unknown."
    };
    snapshot.warning = Some(format!(
        "Showing cached counts. {error}{offset_note}{partial}"
    ));
    snapshot
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
    let cached = cached_snapshot_for_user(settings.get(store::GITHUB_COMMIT_CACHE), &username);
    let now = chrono::Utc::now().timestamp();
    if !refresh.unwrap_or(false) {
        if let Some(cache) = cached
            .as_ref()
            .filter(|cache| cache_is_fresh(cache, &username, today, current_offset, now))
        {
            return Ok(Some(cache.clone()));
        }
    }
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
            let value =
                serde_json::to_value(&snapshot).map_err(|_| "Could not cache GitHub counts.")?;
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
            Some(snapshot) => Ok(Some(cached_failure_snapshot(
                snapshot,
                &error,
                current_offset,
            ))),
            None => Err(error),
        },
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

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
        let cached = snapshot("octocat", -28_800);
        let today = chrono::Local::now().date_naive();
        assert!(cache_is_fresh(&cached, "octocat", today, -28_800, 200));
        assert!(!cache_is_fresh(&cached, "octocat", today, -25_200, 200));

        let stale = cached_failure_snapshot(cached.clone(), "GitHub is unavailable.", -25_200);
        assert_eq!(stale.utc_offset, cached.utc_offset);
        assert_eq!(stale.daily[0].day, cached.daily[0].day);
        assert_eq!(stale.daily[0].commits, 4);
        let warning = stale.warning.unwrap();
        assert!(warning.contains("Showing cached counts"));
        assert!(warning.contains("UTC offset changed"));
        assert!(warning.contains("original daily buckets are retained"));
    }

    #[test]
    fn cached_snapshot_is_only_reused_for_the_matching_account() {
        let value = serde_json::to_value(snapshot("octocat", 0)).unwrap();
        assert!(cached_snapshot_for_user(Some(value.clone()), "OCTOCAT").is_some());
        assert!(cached_snapshot_for_user(Some(value), "other-user").is_none());
    }
}
