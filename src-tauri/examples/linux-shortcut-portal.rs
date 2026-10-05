//! Disposable native portal check. Run outside an editor's inherited app scope.
//! Registers no permanent shortcut and never records audio or reads the clipboard.

#[cfg(target_os = "linux")]
#[path = "../src/core/hotkey/linux_portal.rs"]
mod portal;

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use ashpd::desktop::global_shortcuts::NewShortcut;
    use futures_util::StreamExt;
    use std::process::Command;
    use std::time::Duration;

    let keyboard = std::env::args().any(|arg| arg == "--keyboard");
    let binding_description = format!("Verenu native verification {}", std::process::id());
    for round in 1..=2 {
        let name = format!("verification-{}-{round}", std::process::id());
        let shortcuts = portal::connect()
            .await
            .map_err(|error| anyhow::anyhow!(error.note()))?;
        let session = shortcuts.create_session(Default::default()).await?;
        shortcuts
            .bind_shortcuts(
                &session,
                &[NewShortcut::new(
                    &name,
                    "Verenu native shortcut verification",
                )],
                None,
                Default::default(),
            )
            .await?
            .response()?;
        let listing = Command::new("hyprctl").arg("globalshortcuts").output()?;
        let listing = String::from_utf8(listing.stdout)?;
        let id = listing
            .lines()
            .find_map(|line| {
                let (id, description) = line.split_once(" -> ")?;
                (description.trim() == "Verenu native shortcut verification"
                    && id.trim().ends_with(&format!(":{name}")))
                .then(|| id.trim().to_string())
            })
            .ok_or_else(|| anyhow::anyhow!("Registered probe was absent from Hyprland"))?;
        anyhow::ensure!(
            id.ends_with(&format!(":{name}")) && !id.starts_with("com.t3tools."),
            "Probe inherited another app's identity"
        );
        let mut events = shortcuts.receive_activated().await?;
        let _binding = if keyboard {
            let listing = Command::new("hyprctl").args(["-j", "binds"]).output()?;
            let binds: Vec<serde_json::Value> = serde_json::from_slice(&listing.stdout)?;
            anyhow::ensure!(
                !binds
                    .iter()
                    .any(|binding| binding["description"] != binding_description
                        && binding["modmask"] == 77
                        && (binding["key"]
                            .as_str()
                            .is_some_and(|key| key.eq_ignore_ascii_case("F12"))
                            || binding["catch_all"] == true)),
                "The temporary verification chord is occupied"
            );
            let handle = format!("_verenu_verification_{}_{round}", std::process::id());
            let script = format!("{handle} = hl.bind(\"CTRL + ALT + SHIFT + SUPER + F12\", hl.dsp.global(\"{id}\"), {{ description = \"{binding_description}\", submap_universal = true }})");
            let installed = Command::new("hyprctl").args(["eval", &script]).output()?;
            anyhow::ensure!(
                installed.status.success(),
                "Temporary verification bind failed"
            );
            let binding = TemporaryBinding(handle);
            let pressed = Command::new("wtype")
                .args([
                    "-P",
                    "Control_L",
                    "-P",
                    "Alt_L",
                    "-P",
                    "Shift_L",
                    "-P",
                    "Super_L",
                    "-P",
                    "F12",
                    "-s",
                    "100",
                    "-p",
                    "F12",
                    "-p",
                    "Super_L",
                    "-p",
                    "Shift_L",
                    "-p",
                    "Alt_L",
                    "-p",
                    "Control_L",
                ])
                .output()?;
            anyhow::ensure!(pressed.status.success(), "Wayland keyboard fixture failed");
            Some(binding)
        } else {
            let dispatched = Command::new("hyprctl")
                .args(["dispatch", &format!("hl.dsp.global(\"{id}\")")])
                .output()?;
            anyhow::ensure!(dispatched.status.success(), "Compositor dispatch failed");
            None
        };
        let activation = tokio::time::timeout(Duration::from_secs(5), events.next()).await;
        drop(_binding);
        session.close().await?;
        let event = activation?.ok_or_else(|| anyhow::anyhow!("Activation stream ended"))?;
        anyhow::ensure!(event.shortcut_id() == name, "Wrong shortcut activated");
        println!("round={round} registered_identity=true session=true compositor_activation=true keyboard={keyboard}");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
struct TemporaryBinding(String);

#[cfg(target_os = "linux")]
impl Drop for TemporaryBinding {
    fn drop(&mut self) {
        let _ = std::process::Command::new("hyprctl")
            .args(["eval", &format!("{}:set_enabled(false)", self.0)])
            .output();
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This probe requires Linux with a Hyprland desktop portal.");
    std::process::exit(2);
}
