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

    pub fn everywhere() -> Self {
        Self {
            id: db::EVERYWHERE_CONTEXT_ID,
            label: "Everywhere".to_string(),
        }
    }
}

pub fn resolve_context(db: &Db, executable: &str, domain: Option<&str>) -> Result<Context> {
    // App bundles/installers commonly replace their executable name on every
    // update. Refresh a target lazily on the dictation path so users do not
    // need to reopen the Contexts screen after a nightly release changes.
    let installed_apps = crate::system::apps::list_installed_apps_cached();
    db::reconcile_context_targets(db, &installed_apps)?;
    db::resolve_context_for_target(db, executable, domain)
}

#[cfg(test)]
mod tests {
    use super::*;

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
