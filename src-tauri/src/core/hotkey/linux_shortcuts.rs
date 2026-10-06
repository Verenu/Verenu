//! Omarchy-visible auxiliary shortcuts and temporary dictation controls.

use std::{fs, path::PathBuf};

pub(super) const START: &str = "-- >>> Verenu copy last dictation (do not edit) <<<";
pub(super) const END: &str = "-- <<< End Verenu copy last dictation >>>";

pub(super) fn ensure_copy_binding(portal_id: &str, chord: &str) -> Result<(), String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("XDG config directory is unavailable")?;
    let path = config.join("hypr/bindings.lua");
    let current = fs::read_to_string(&path).map_err(|e| format!("cannot read bindings: {e}"))?;
    let next = copy_config(&current, portal_id, chord)?;
    if next != current {
        let tmp = path.with_extension("lua.verenu-copy.tmp");
        fs::write(&tmp, next).map_err(|e| format!("cannot write bindings: {e}"))?;
        fs::rename(&tmp, &path).map_err(|e| format!("cannot replace bindings: {e}"))?;
    }
    // Resolve the action again even when a reconnect kept the same portal ID.
    let output = std::process::Command::new("hyprctl")
        .arg("reload")
        .output()
        .map_err(|e| format!("Hyprland reload failed: {e}"))?;
    if !output.status.success() {
        return Err("Hyprland rejected the copy shortcut".into());
    }
    let errors = std::process::Command::new("hyprctl")
        .args(["-j", "configerrors"])
        .output()
        .map_err(|e| format!("cannot validate Hyprland bindings: {e}"))?;
    let errors: Vec<String> = serde_json::from_slice(&errors.stdout)
        .map_err(|_| "cannot read Hyprland config validation".to_string())?;
    if errors.iter().any(|error| !error.trim().is_empty()) {
        return Err("Hyprland reports configuration errors after shortcut registration".into());
    }
    Ok(())
}

fn copy_config(current: &str, portal_id: &str, chord: &str) -> Result<String, String> {
    let id = lua_string(portal_id);
    let chord = super::super::chord::Chord::parse(chord)
        .ok_or("Invalid copy shortcut")?
        .hyprland();
    let chord = lua_string(&chord);
    let block = format!(
        "{START}\n_verenu_copy_binding = hl.bind({chord}, hl.dsp.global({id}), {{ description = \"Verenu copy last dictation\", submap_universal = true }})\n{END}"
    );
    match (current.find(START), current.find(END)) {
        (Some(a), Some(b)) if a < b => Ok(format!(
            "{}{}{}",
            &current[..a],
            block,
            &current[b + END.len()..]
        )),
        (None, None) => Ok(format!("{}\n\n{block}\n", current.trim_end())),
        _ => Err("Verenu copy shortcut block has incomplete markers".into()),
    }
}

pub(super) fn remove_block(start: &str, end: &str) -> Result<(), String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("XDG config directory is unavailable")?;
    let path = config.join("hypr/bindings.lua");
    let current = fs::read_to_string(&path).map_err(|_| "Cannot read desktop bindings")?;
    if current.contains(start) != current.contains(end) {
        return Err("Incomplete Verenu shortcut block".into());
    }
    if let (Some(a), Some(b)) = (current.find(start), current.find(end)) {
        if a >= b {
            return Err("Incomplete Verenu shortcut block".into());
        }
        let next = format!("{}{}", &current[..a], &current[b + end.len()..]);
        let tmp = path.with_extension("lua.verenu-remove.tmp");
        fs::write(&tmp, next).map_err(|_| "Cannot update desktop bindings")?;
        fs::rename(tmp, path).map_err(|_| "Cannot replace desktop bindings")?;
        let output = std::process::Command::new("hyprctl")
            .arg("reload")
            .output()
            .map_err(|_| "Cannot reload desktop bindings")?;
        if !output.status.success() {
            return Err("Cannot reload desktop bindings".into());
        }
    }
    Ok(())
}

fn lua_string(value: &str) -> String {
    // Portal IDs cannot contain controls; JSON quoting also handles slashes
    // and quotes in compositor-provided IDs without Lua interpolation.
    serde_json::to_string(value).expect("string serialization")
}

pub(super) fn temporary_binding(
    name: &str,
    key: &str,
    id: Option<&str>,
    description: &str,
) -> String {
    let handle = format!("_verenu_{name}_binding");
    let portal_id = format!("_verenu_{name}_portal_id");
    let binding_key = format!("_verenu_{name}_key");
    // Hyprland 0.56's handle:unbind() removes every binding on the same
    // trigger. An expired handle can then crash its Lua accessor. Keep our
    // handle alive and disable it instead; a config reload clears it safely.
    let disable = format!(
        "if {handle} and tostring({handle}) ~= \"HL.Keybind(expired)\" then {handle}:set_enabled(false) else {handle} = nil end"
    );
    match id {
        Some(id) => format!(
            "{disable}; if not {handle} or {portal_id} ~= {id} or {binding_key} ~= {key} then {handle} = hl.bind({key}, hl.dsp.global({id}), {{ description = {description}, ignore_mods = true, submap_universal = true }}); {portal_id} = {id}; {binding_key} = {key} end; {handle}:set_enabled(true)",
            key = lua_string(key), id = lua_string(id), description = lua_string(description)
        ),
        None => disable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a Lua interpreter; executes generated control lifecycle"]
    fn generated_lua_controls_replace_changed_keys_and_disable_unavailable_handles() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut fixture = include_str!("../../../../tests/fixtures/hyprland-temporary-controls.lua").to_string();
        for (marker, key, id) in [
            ("INITIAL_CONTROL", "F8", Some("app:cancel")),
            ("CHANGED_CONTROL", "ESCAPE", Some("app:cancel")),
            ("UNAVAILABLE_CONTROL", "", None),
            ("RESTORED_CONTROL", "ESCAPE", Some("app:cancel")),
            ("RECONNECTED_CONTROL", "ESCAPE", Some("new:cancel")),
            ("DISCONNECTED_CONTROL", "", None),
        ] {
            fixture = fixture.replace(&format!("-- {marker}"), &temporary_binding("escape", key, id, "Verenu cancel dictation"));
        }
        let mut child = Command::new("lua").arg("-").stdin(Stdio::piped())
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
            .expect("Lua interpreter is required for this fixture");
        child.stdin.take().unwrap().write_all(fixture.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn copy_registration_is_idempotent_and_preserves_personal_bindings() {
        let personal = "-- personal\no.bind(\"SUPER + B\", \"Browser\", \"browser\")\n";
        let first = copy_config(personal, "app:copy", "Ctrl+Alt+C").unwrap();
        assert!(first.starts_with(personal));
        assert!(first.contains("hl.bind(\"CTRL + ALT + C\""));
        assert_eq!(
            copy_config(&first, "app:copy", "Ctrl+Alt+C").unwrap(),
            first
        );
        let changed = copy_config(&first, "new:copy", "Ctrl+Alt+F6").unwrap();
        assert!(changed.contains("CTRL + ALT + F6"));
        assert_eq!(changed.matches(START).count(), 1);
        assert!(!changed.contains("app:copy"));
        assert!(changed.contains("new:copy"));
        assert!(copy_config(START, "app:copy", "Ctrl+Alt+C").is_err());
        assert!(copy_config(&format!("{END}\n{START}"), "app:copy", "Ctrl+Alt+C").is_err());
    }

    #[test]
    fn temporary_controls_disable_only_their_own_handles_and_accept_held_modifiers() {
        let armed = temporary_binding(
            "escape",
            "ESCAPE",
            Some("app:cancel"),
            "Verenu cancel dictation",
        );
        assert!(armed.contains("ignore_mods = true"));
        assert!(armed.contains("_verenu_escape_binding:set_enabled(false)"));
        assert!(armed.contains("_verenu_escape_binding:set_enabled(true)"));
        assert!(armed.contains("HL.Keybind(expired)"));
        assert!(!armed.contains(":unbind()"));
        assert!(!armed.contains("hl.unbind"));
        let disarmed = temporary_binding("escape", "ESCAPE", None, "");
        assert!(!disarmed.contains("hl.bind"));
        assert_eq!(disarmed, temporary_binding("escape", "F8", None, ""));
    }
}
