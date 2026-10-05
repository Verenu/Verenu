//! Each shortcut session owns its portal identity and D-Bus connection.

use ashpd::desktop::global_shortcuts::GlobalShortcuts;
use std::future::Future;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

// Match installed launchers, including the Arch package and AppImage integration.
// Never inherit the identity of the terminal or editor that launched Verenu.
pub(super) fn desktop_id() -> &'static str {
    ["com.verenu.app", "verenu", "Verenu"]
        .into_iter()
        .find(|id| gtk::gio::DesktopAppInfo::new(&format!("{id}.desktop")).is_some())
        .unwrap_or("verenu")
}

pub(super) async fn request<T, E: std::fmt::Display>(
    stage: &'static str,
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, Failure> {
    match tokio::time::timeout(REQUEST_TIMEOUT, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(Failure::new(stage, error.to_string())),
        Err(_) => Err(Failure::new(
            stage,
            "The desktop portal did not respond within 10 seconds",
        )),
    }
}

pub(super) async fn connect() -> Result<GlobalShortcuts, Failure> {
    // A fresh connection lets reconnects register again. Registration must
    // precede all portal calls, and must use the same connection as shortcuts.
    let connection = request("Connecting to the desktop", zbus::Connection::session()).await?;
    if !ashpd::is_sandboxed() {
        let registry = request(
            "Registering Verenu's app identity",
            zbus::Proxy::new(
                &connection,
                "org.freedesktop.portal.Desktop",
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.host.portal.Registry",
            ),
        )
        .await?;
        let options = std::collections::HashMap::<&str, zbus::zvariant::Value<'_>>::new();
        let result = request("Registering Verenu's app identity", async {
            // Registry also accepts legacy desktop basenames such as `verenu`,
            // which ashpd::AppID deliberately restricts to reverse-DNS names.
            registry
                .call::<_, _, ()>("Register", &(desktop_id(), options))
                .await
        })
        .await;
        if let Err(error) = result {
            // Registry was introduced in portal 1.20. Older portals still
            // discover host identities themselves; only a missing API permits
            // that legacy path, never a rejected identity or an outage.
            if !error.legacy_registry_missing() {
                return Err(error);
            }
            log::info!("linux hotkey: desktop portal uses legacy app identity discovery");
        }
    }
    request(
        "Opening global shortcuts",
        GlobalShortcuts::with_connection(connection),
    )
    .await
}

#[derive(Debug)]
pub(super) struct Failure {
    pub stage: &'static str,
    pub reason: String,
}

impl Failure {
    pub fn new(stage: &'static str, reason: impl Into<String>) -> Self {
        Self {
            stage,
            reason: reason.into(),
        }
    }

    fn legacy_registry_missing(&self) -> bool {
        self.reason
            .contains("org.freedesktop.DBus.Error.UnknownMethod")
            || self
                .reason
                .contains("org.freedesktop.DBus.Error.UnknownInterface")
    }

    pub fn note(&self) -> String {
        let action = if matches!(
            self.stage,
            "Reading dictation shortcut" | "Finding available shortcut"
        ) {
            "Choose another dictation hotkey in Settings > General or change the conflicting desktop binding."
        } else if self.reason.contains("An app id is required")
            || self.stage == "Registering Verenu's app identity"
        {
            "The desktop portal could not identify Verenu. Reinstall Verenu and restart the app."
        } else if self.reason.contains("Cancelled") || self.reason.contains("NotAllowed") {
            "Allow Verenu in the desktop shortcut prompt, then restart the app."
        } else {
            "Check that xdg-desktop-portal and the Hyprland portal backend are installed and running."
        };
        let retry = if matches!(
            self.stage,
            "Starting shortcut listener" | "Reading dictation shortcut"
        ) {
            "Restart Verenu to try again."
        } else {
            "Verenu will retry automatically."
        };
        format!(
            "Global shortcuts are unavailable. {action} {retry} Details: {}: {}",
            self.stage, self.reason
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Failure;

    #[test]
    fn missing_identity_reports_the_cause_and_repair_instead_of_a_conflict() {
        let note = Failure::new("Creating shortcut session", "An app id is required").note();
        assert!(note.contains("could not identify Verenu"));
        assert!(note.contains("Reinstall Verenu"));
        assert!(note.contains("An app id is required"));
        assert!(note.contains("retry automatically"));
        assert!(!note.contains("choose another shortcut"));
    }

    #[test]
    fn only_a_missing_registry_api_allows_legacy_identity_discovery() {
        for name in ["UnknownMethod", "UnknownInterface"] {
            assert!(Failure::new(
                "Registering",
                format!("org.freedesktop.DBus.Error.{name}: missing")
            )
            .legacy_registry_missing());
        }
        for reason in [
            "An app id is required",
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "Connection already associated with an application ID",
            "timeout",
        ] {
            assert!(!Failure::new("Registering", reason).legacy_registry_missing());
        }
    }

    #[tokio::test]
    async fn request_preserves_the_failing_stage_and_reason() {
        let error = super::request("Creating shortcut session", async {
            Err::<(), _>("NotAllowed: denied")
        })
        .await
        .unwrap_err();
        assert_eq!(error.stage, "Creating shortcut session");
        assert!(error.note().contains("Allow Verenu"));
        assert!(error.note().contains("NotAllowed: denied"));
    }
}
