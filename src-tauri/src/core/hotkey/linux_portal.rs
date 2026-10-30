use ashpd::desktop::global_shortcuts::GlobalShortcuts;
use std::future::Future;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

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

pub(super) async fn connect() -> Result<(GlobalShortcuts, zbus::Connection), Failure> {
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
    let shortcuts = request(
        "Opening global shortcuts",
        GlobalShortcuts::with_connection(connection.clone()),
    )
    .await?;
    Ok((shortcuts, connection))
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

    #[cfg(feature = "native-testing")]
    struct Registry;

    #[cfg(feature = "native-testing")]
    #[zbus::interface(name = "org.freedesktop.host.portal.Registry")]
    impl Registry {
        fn register(
            &self,
            _app_id: &str,
            _options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        ) {
        }
    }

    #[cfg(feature = "native-testing")]
    struct Shortcuts;

    #[cfg(feature = "native-testing")]
    #[zbus::interface(name = "org.freedesktop.portal.GlobalShortcuts")]
    impl Shortcuts {
        #[zbus(property)]
        fn version(&self) -> u32 {
            1
        }
    }

    #[cfg(feature = "native-testing")]
    #[tokio::test]
    async fn closing_portal_connection_disconnects_even_with_a_live_proxy() {
        let _portal = zbus::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .serve_at("/org/freedesktop/portal/desktop", Registry)
            .unwrap()
            .serve_at("/org/freedesktop/portal/desktop", Shortcuts)
            .unwrap()
            .build()
            .await
            .unwrap();
        let (proxy, connection) = super::connect().await.unwrap();
        let name = connection.unique_name().unwrap().to_string();
        let observer = zbus::Connection::session().await.unwrap();
        async fn has_owner(observer: &zbus::Connection, name: &str) -> bool {
            observer
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "NameHasOwner",
                    &(name,),
                )
                .await
                .unwrap()
                .body()
                .deserialize()
                .unwrap()
        }
        assert!(has_owner(&observer, &name).await);
        connection.close().await.unwrap();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while has_owner(&observer, &name).await {
            assert!(
                tokio::time::Instant::now() < deadline,
                "portal connection did not close"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        drop(proxy);
    }

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
