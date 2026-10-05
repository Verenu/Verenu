//! Foreground executable to content-context resolution.

use anyhow::Result;

use crate::data::db::{self, Context, Db};

/// The portion of a resolved Context that must survive asynchronous pipeline
/// work.  Keep this separate from the full database row: an AutoLearn
/// monitor only needs the stable identity and a safe display label for
/// redacted diagnostics.  In particular, it must never reconstruct a Context
/// later from the foreground executable or browser process.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResolvedContextIdentity {
    pub id: i64,
    pub label: String,
}

impl ResolvedContextIdentity {
    pub fn from_context(context: &Context) -> Self {
        Self {
            id: context.id,
            label: context.name.clone(),
        }
    }

    /// Pill/diagnostic label for a sub-app match: "Work · #design".
    pub fn from_context_and_sub_app(context: &Context, sub_app: Option<&db::ContextSubApp>) -> Self {
        let mut identity = Self::from_context(context);
        if let Some(sub_app) = sub_app {
            identity.label = format!("{} · {}", context.name, sub_app.label);
        }
        identity
    }

    pub fn everywhere() -> Self {
        Self {
            id: db::EVERYWHERE_CONTEXT_ID,
            label: "Everywhere".to_string(),
        }
    }
}

#[cfg(test)]
fn resolve_context(db: &Db, executable: &str, domain: Option<&str>) -> Result<Context> {
    resolve_context_with_title(db, executable, domain, None).map(|(context, _)| context)
}

/// Like [`resolve_context`], but a window title lets a sub-app rule win over
/// website and app targets.
pub fn resolve_context_with_title(
    db: &Db,
    executable: &str,
    domain: Option<&str>,
    window_title: Option<&str>,
) -> Result<(Context, Option<db::ContextSubApp>)> {
    // App bundles/installers commonly replace their executable name on every
    // update. Refresh a target lazily on the dictation path so users do not
    // need to reopen the Contexts screen after a nightly release changes.
    let installed_apps = crate::system::apps::list_installed_apps_cached_with_status().0;
    db::reconcile_context_targets(db, &installed_apps)?;
    db::resolve_context_with_sub_app(db, executable, domain, window_title)
}

/// Linux window classes can differ from the executable targets already saved
/// in Contexts. Prefer the class/domain match, then try the captured process's
/// executable basename. Never inspect the live foreground for this fallback.
#[cfg(test)]
fn resolve_context_for_window(
    db: &Db,
    executable: &str,
    domain: Option<&str>,
    target_id: usize,
) -> Result<Context> {
    resolve_context_for_captured_window(db, executable, domain, None, target_id)
        .map(|(context, _)| context)
}

pub fn resolve_context_for_captured_window(
    db: &Db,
    executable: &str,
    domain: Option<&str>,
    title: Option<&str>,
    target_id: usize,
) -> Result<(Context, Option<db::ContextSubApp>)> {
    let resolved = resolve_context_with_title(db, executable, domain, title)?;
    #[cfg(target_os = "linux")]
    {
        if resolved.1.is_some() {
            return Ok(resolved);
        }
        let alias = crate::core::window_context::linux_executable_for_pid(target_id);
        let alias = alias
            .as_deref()
            .filter(|alias| !alias.trim().eq_ignore_ascii_case(executable.trim()));
        resolve_alias_with_title(db, executable, resolved, alias, domain, title)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = target_id;
        Ok(resolved)
    }
}

#[cfg(any(target_os = "linux", test))]
fn resolve_alias_with_title(
    db: &Db,
    executable: &str,
    resolved: (Context, Option<db::ContextSubApp>),
    alias: Option<&str>,
    domain: Option<&str>,
    title: Option<&str>,
) -> Result<(Context, Option<db::ContextSubApp>)> {
    if resolved.1.is_some() {
        return Ok(resolved);
    }
    let Some(alias) = alias.filter(|value| !value.trim().is_empty()) else {
        return Ok(resolved);
    };
    if alias.trim().eq_ignore_ascii_case(executable.trim()) {
        return Ok(resolved);
    }
    let title = title.filter(|value| !value.trim().is_empty());
    if !resolved.0.is_everywhere && title.is_none() {
        return Ok(resolved);
    }
    let alias_match = db::resolve_context_with_sub_app(db, alias, domain, title)?;
    // A sub-app is more specific than a website/app. Otherwise keep
    // a class or website assignment ahead of an executable alias.
    if alias_match.1.is_some() || resolved.0.is_everywhere {
        Ok(alias_match)
    } else {
        Ok(resolved)
    }
}

#[cfg(test)]
fn resolve_executable_alias(db: &Db, context: Context, alias: Option<&str>) -> Result<Context> {
    resolve_alias_with_title(db, "", (context, None), alias, None, None)
        .map(|(context, _)| context)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_alias_sub_app_wins_over_website_but_keeps_explicit_class_sub_app() {
        let db = db::open(":memory:").unwrap();
        let coding = db::insert_context_returning(&db, "AI Coding", None, None, None, None, false).unwrap();
        let website = db::insert_context_returning(&db, "Website", None, None, None, None, false).unwrap();
        db::assign_context_website(&db, website.id, "example.com").unwrap();
        let sub = db::create_sub_app(&db, db::NewSubApp {
            executable: "t3code", app_name: None, label: "Project", icon: None,
            title_pattern: "Verenu", match_mode: db::TitleMatchMode::Contains,
        }).unwrap();
        db::assign_sub_app(&db, sub.id, Some(coding.id)).unwrap();
        let base = db::resolve_context_with_sub_app(&db, "com.t3tools.T3Code", Some("example.com"), Some("Verenu - T3 Code")).unwrap();
        let resolved = resolve_alias_with_title(
            &db,
            "com.t3tools.T3Code",
            base,
            Some("t3code"),
            Some("example.com"),
            Some("Verenu - T3 Code"),
        )
        .unwrap();
        assert_eq!(resolved.0.id, coding.id);
        assert_eq!(resolved.1.unwrap().id, sub.id);
        let explicit = db::create_sub_app(&db, db::NewSubApp {
            executable: "com.t3tools.T3Code", app_name: None, label: "Explicit", icon: None,
            title_pattern: "Verenu", match_mode: db::TitleMatchMode::Contains,
        }).unwrap();
        db::assign_sub_app(&db, explicit.id, Some(website.id)).unwrap();
        let base = db::resolve_context_with_sub_app(&db, "com.t3tools.T3Code", Some("example.com"), Some("Verenu - T3 Code")).unwrap();
        let resolved = resolve_alias_with_title(
            &db,
            "com.t3tools.T3Code",
            base,
            Some("t3code"),
            Some("example.com"),
            Some("Verenu - T3 Code"),
        )
        .unwrap();
        assert_eq!(resolved.1.unwrap().id, explicit.id);
    }

    #[test]
    fn executable_alias_matches_saved_linux_target_without_overriding_class_or_website() {
        let db = db::open(":memory:").expect("db");
        let coding = db::insert_context_returning(&db, "AI Coding", None, None, None, None, false)
            .expect("coding context");
        db::assign_context_target(&db, coding.id, "t3code").expect("executable target");
        let fallback = db::resolve_context_for_target(&db, "com.t3tools.T3Code", None).unwrap();
        let resolved = resolve_executable_alias(&db, fallback.clone(), Some("T3CODE")).unwrap();
        assert_eq!(
            ResolvedContextIdentity::from_context(&resolved).label,
            "AI Coding"
        );
        for alias in [None, Some(""), Some("unknown")] {
            assert!(
                resolve_executable_alias(&db, fallback.clone(), alias)
                    .unwrap()
                    .is_everywhere
            );
        }

        let specific = db::insert_context_returning(&db, "Specific", None, None, None, None, false)
            .expect("specific context");
        db::assign_context_target(&db, specific.id, "com.t3tools.T3Code").unwrap();
        let class_match = db::resolve_context_for_target(&db, "com.t3tools.T3Code", None).unwrap();
        assert_eq!(
            resolve_executable_alias(&db, class_match, Some("t3code"))
                .unwrap()
                .id,
            specific.id
        );
        db::assign_context_website(&db, specific.id, "example.com").unwrap();
        let website_match =
            db::resolve_context_for_target(&db, "unknown", Some("example.com")).unwrap();
        assert_eq!(
            resolve_executable_alias(&db, website_match, Some("t3code"))
                .unwrap()
                .id,
            specific.id
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn window_context_uses_captured_pid_executable_and_handles_missing_pid() {
        let db = db::open(":memory:").expect("db");
        let coding =
            db::insert_context_returning(&db, "AI Coding", None, None, None, None, false).unwrap();
        let executable = std::env::current_exe().unwrap();
        db::assign_context_target(
            &db,
            coding.id,
            executable.file_name().unwrap().to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            resolve_context_for_window(
                &db,
                "synthetic.window.class",
                None,
                std::process::id() as usize
            )
            .unwrap()
            .id,
            coding.id
        );
        for pid in [0, usize::MAX] {
            assert!(
                resolve_context_for_window(&db, "synthetic.window.class", None, pid)
                    .unwrap()
                    .is_everywhere
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a live Hyprland client PID in VERENU_CONTEXT_FIXTURE_PID"]
    fn live_linux_window_matches_executable_context() {
        let pid: usize = std::env::var("VERENU_CONTEXT_FIXTURE_PID")
            .expect("fixture PID")
            .parse()
            .expect("numeric PID");
        let class = crate::core::window_context::get_process_name_for_hwnd(pid)
            .expect("captured window class");
        let executable =
            std::fs::read_link(format!("/proc/{pid}/exe")).expect("captured executable");
        let basename = executable.file_name().unwrap().to_str().unwrap();
        assert!(
            !class.eq_ignore_ascii_case(basename),
            "fixture needs differing identities"
        );
        let db = db::open(":memory:").expect("isolated db");
        let coding =
            db::insert_context_returning(&db, "AI Coding", None, None, None, None, false).unwrap();
        db::assign_context_target(&db, coding.id, basename).unwrap();
        let context = resolve_context_for_window(&db, &class, None, pid).unwrap();
        assert_eq!(
            ResolvedContextIdentity::from_context(&context).label,
            "AI Coding"
        );
    }

    #[test]
    fn resolver_matches_executables_case_insensitively() {
        let db = db::open(":memory:").expect("db");
        let context = db::insert_context_returning(&db, "Editor", None, None, None, None, false)
            .expect("context");
        db::assign_context_target(&db, context.id, "editor.exe").expect("target");

        assert_eq!(
            resolve_context(&db, "EDITOR.EXE", None).unwrap().id,
            context.id
        );
    }

    #[test]
    fn resolver_uses_everywhere_for_unknown_executable() {
        let db = db::open(":memory:").expect("db");

        let context = resolve_context(&db, "unknown.exe", None).expect("fallback");
        assert_eq!(context.id, db::EVERYWHERE_CONTEXT_ID);
        assert!(context.is_everywhere);
    }

    #[test]
    fn resolved_context_identity_keeps_only_stable_id_and_display_label() {
        let context = Context {
            id: 42,
            name: "Development".to_string(),
            is_everywhere: false,
            icon: None,
            tone: None,
            cleanup_intensity: None,
            color: None,
            custom_instructions: None,
            contextual_formatting_disabled: false,
        paste_in_chunks: false,
            pinned_at: None,
            created_at: String::new(),
            updated_at: String::new(),
        };

        assert_eq!(
            ResolvedContextIdentity::from_context(&context),
            ResolvedContextIdentity {
                id: 42,
                label: "Development".to_string(),
            }
        );
    }
}
