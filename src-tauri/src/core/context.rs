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
pub fn resolve_context(db: &Db, executable: &str, domain: Option<&str>) -> Result<Context> {
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
pub fn resolve_context_for_window(
    db: &Db,
    executable: &str,
    domain: Option<&str>,
    target_id: usize,
) -> Result<Context> {
    let context = resolve_context(db, executable, domain)?;
    #[cfg(target_os = "linux")]
    {
        if !context.is_everywhere {
            return Ok(context);
        }
        let alias = u32::try_from(target_id)
            .ok()
            .filter(|pid| *pid != 0)
            .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok())
            .and_then(|path| path.file_name()?.to_str().map(str::to_owned));
        resolve_executable_alias(db, context, alias.as_deref())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = target_id;
        Ok(context)
    }
}

#[cfg(any(target_os = "linux", test))]
fn resolve_executable_alias(db: &Db, context: Context, alias: Option<&str>) -> Result<Context> {
    if context.is_everywhere {
        if let Some(alias) = alias.filter(|value| !value.trim().is_empty()) {
            return db::resolve_context_for_target(db, alias, None);
        }
    }
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::*;

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
