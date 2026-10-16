use super::*;

struct Controls(Vec<String>);
impl Drop for Controls {
    fn drop(&mut self) {
        for name in &self.0 {
            let _ = eval_hyprland(&shortcuts::temporary_binding(name, "", None, ""));
        }
    }
}

#[tokio::test]
#[ignore = "requires the real Hyprland desktop portal; uses no keyboard injection"]
async fn secondary_controls_register_selected_keys_and_receive_native_actions() {
    let shortcuts = portal::connect().await.expect("desktop portal");
    let session = shortcuts.create_session(Default::default()).await.unwrap();
    let prefix = format!("secondary-probe-{}", std::process::id());
    let actions = [format!("{prefix}-cancel"), format!("{prefix}-handsfree")];
    let descriptions = [
        format!("Verenu cancel dictation (verification {prefix})"),
        format!("Verenu switch to hands-free (verification {prefix})"),
    ];
    shortcuts.bind_shortcuts(&session, &[
        NewShortcut::new(&actions[0], &descriptions[0]),
        NewShortcut::new(&actions[1], &descriptions[1]),
    ], None, Default::default()).await.unwrap().response().unwrap();
    let listing = std::process::Command::new("hyprctl").arg("globalshortcuts").output().unwrap();
    let listing = String::from_utf8(listing.stdout).unwrap();
    let mut events = shortcuts.receive_activated().await.unwrap();
    let mut controls = Controls(vec![]);
    let mut chosen = Vec::new();
    for index in 0..2 {
        let mut desktop = conflicts::bindings("secondary-probe").unwrap();
        for binding in &mut desktop { binding.description.clear(); }
        let key = ["Quote", "Semicolon", "BracketLeft", "BracketRight", "Backslash", "Minus", "Equal"]
            .iter().filter_map(|code| control_trigger(code).ok())
            .find(|key| !chosen.contains(key) && conflicts::control_key(&desktop, key).is_some())
            .expect("two unused single keys are required; occupied keys are never replaced");
        let id = listing.lines().find_map(|line| {
            let (id, description) = line.split_once(" -> ")?;
            (description.trim() == descriptions[index] && id.ends_with(&format!(":{}", actions[index])))
                .then(|| id.trim().to_string())
        }).expect("own scoped portal action");
        let name = format!("secondary_probe_{}_{}", std::process::id(), index);
        controls.0.push(name.clone());
        let snippet = shortcuts::temporary_binding(&name, &key, Some(&id), &descriptions[index]);
        eval_hyprland(&snippet).unwrap();
        let binds = std::process::Command::new("hyprctl").args(["-j", "binds"]).output().unwrap();
        let binds: Vec<serde_json::Value> = serde_json::from_slice(&binds.stdout).unwrap();
        assert!(binds.iter().any(|bind| bind["description"] == descriptions[index]
            && bind["key"].as_str().is_some_and(|value| value.eq_ignore_ascii_case(&key))));
        let result = std::process::Command::new("hyprctl")
            .args(["dispatch", &format!("hl.dsp.global({})", serde_json::to_string(&id).unwrap())])
            .output().unwrap();
        assert!(result.status.success());
        let event = tokio::time::timeout(Duration::from_secs(5), events.next()).await.unwrap().unwrap();
        assert_eq!(event.shortcut_id(), actions[index]);
        eval_hyprland(&shortcuts::temporary_binding(&name, "", None, "")).unwrap();
        chosen.push(key);
    }
    drop(controls);
    session.close().await.unwrap();
}
