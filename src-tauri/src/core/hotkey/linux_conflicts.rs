use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct Binding {
    #[serde(default)]
    pub modmask: u32,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub keycode: i32,
    #[serde(default)]
    pub catch_all: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub ignore_mods: bool,
}

pub(super) fn bindings(action: &str) -> Result<Vec<Binding>, String> {
    let output = std::process::Command::new("hyprctl")
        .args(["-j", "binds"])
        .output()
        .map_err(|_| "Cannot check desktop shortcuts".to_string())?;
    if !output.status.success() {
        return Err("Cannot check desktop shortcuts".into());
    }
    let mut bindings: Vec<Binding> = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Cannot read desktop shortcuts".to_string())?;
    for status in super::super::shortcut_status::get_shortcut_status() {
        if status.id == action {
            continue;
        }
        let Some(active) = status.active else {
            continue;
        };
        let triggers = if status.id == "dictation" {
            super::shortcut_configuration_for(
                &status
                    .codes
                    .iter()
                    .map(|code| super::map_code_to_vk(code))
                    .collect::<Vec<_>>(),
            )
            .map(|(_, triggers)| triggers)
            .unwrap_or_default()
        } else {
            vec![active]
        };
        for trigger in triggers {
            let (modmask, key) = parse(&trigger);
            bindings.push(Binding {
                modmask,
                key,
                ignore_mods: matches!(status.id.as_str(), "cancel" | "handsfree"),
                ..Binding::default()
            });
        }
    }
    Ok(bindings)
}

fn owned(description: &str) -> bool {
    [
        "Verenu dictation",
        "Verenu copy last dictation",
        "Verenu capture sub-app",
        "Verenu cancel dictation",
        "Verenu switch to hands-free",
    ]
    .iter()
    .any(|name| {
        description
            .strip_prefix(name)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(" ("))
    })
}

fn parse(trigger: &str) -> (u32, String) {
    let plus_key = trigger.trim_end().ends_with("++");
    let trigger = if plus_key { trigger.trim_end().trim_end_matches('+') } else { trigger };
    let mut parts = trigger.split('+').map(str::trim).collect::<Vec<_>>();
    let key = if plus_key { "+".to_string() } else { parts.pop().unwrap_or_default().to_ascii_uppercase() };
    let mask = parts.iter().fold(0, |mask, part| {
        mask | match part.to_ascii_uppercase().as_str() {
            "SHIFT" => 1,
            "CTRL" | "CONTROL" => 4,
            "ALT" => 8,
            "SUPER" => 64,
            _ => 0,
        }
    });
    (mask, key)
}

fn keycode(key: &str) -> Option<u32> {
    match key {
        "CONTROL_L" => Some(37),
        "CONTROL_R" => Some(105),
        "SUPER_L" => Some(133),
        "SUPER_R" => Some(134),
        "ALT_L" => Some(64),
        "ALT_R" => Some(108),
        "SHIFT_L" => Some(50),
        "SHIFT_R" => Some(62),
        "ESCAPE" => Some(9),
        "SPACE" => Some(65),
        "C" => Some(54),
        "S" => Some(39),
        "A" => Some(38),
        "B" => Some(56),
        "D" => Some(40),
        "E" => Some(26),
        "F" => Some(41),
        "G" => Some(42),
        "H" => Some(43),
        "I" => Some(31),
        "J" => Some(44),
        "K" => Some(45),
        "L" => Some(46),
        "M" => Some(58),
        "N" => Some(57),
        "O" => Some(32),
        "P" => Some(33),
        "Q" => Some(24),
        "R" => Some(27),
        "T" => Some(28),
        "U" => Some(30),
        "V" => Some(55),
        "W" => Some(25),
        "X" => Some(53),
        "Y" => Some(29),
        "Z" => Some(52),
        "0" => Some(19),
        _ if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() => {
            key.parse::<u32>().ok().map(|n| 9 + n)
        }
        _ => super::REGULAR_KEYS
            .iter()
            .find(|(_, name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, _, code)| *code)
            .or_else(|| {
                key.strip_prefix('F')
                    .and_then(|n| n.parse::<u32>().ok())
                    .and_then(|n| match n {
                        1..=10 => Some(66 + n),
                        11..=12 => Some(84 + n),
                        _ => None,
                    })
            }),
    }
}

pub(super) fn free(bindings: &[Binding], triggers: &[String], ignore_mods: bool) -> bool {
    triggers.iter().all(|trigger| {
        matching_binding(bindings, trigger, ignore_mods).is_none()
    })
}

fn matching_binding<'a>(bindings: &'a [Binding], trigger: &str, ignore_mods: bool) -> Option<&'a Binding> {
    let (mask, key) = parse(trigger);
    bindings.iter().filter(|b| !owned(&b.description)).find(|b| {
        (ignore_mods || b.ignore_mods || b.modmask == mask)
            && (b.catch_all || b.key.eq_ignore_ascii_case(&key)
                || (b.keycode > 0 && keycode(&key) == Some(b.keycode as u32)))
    })
}

pub(super) fn control_conflict(id: &str, requested: &str, bindings: &[Binding], keys: &[u32]) -> Option<super::super::shortcut_status::ShortcutConflict> {
    let mut parts: Vec<&str> = keys.iter().filter_map(|key| super::portal_modifier_name(*key)).collect();
    parts.push(requested);
    let held = parts.join(" + ");
    let candidates = if id == "handsfree" { vec![held.as_str()] } else { vec![requested, held.as_str()] };
    let (trigger, binding) = candidates.into_iter().find_map(|trigger| matching_binding(bindings, trigger, false).map(|binding| (trigger, binding)))?;
    Some(super::super::shortcut_status::ShortcutConflict {
        trigger: trigger.into(),
        description: (!binding.description.is_empty()).then(|| binding.description.clone()),
        desktop: std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "Hyprland".into()),
        while_holding: trigger == held && held != requested,
    })
}

pub(super) fn choose(
    bindings: &[Binding],
    requested: &str,
    alternatives: &[&str],
    ignore_mods: bool,
) -> Option<String> {
    std::iter::once(requested)
        .chain(alternatives.iter().copied())
        .find(|trigger| free(bindings, &[trigger.to_string()], ignore_mods))
        .map(str::to_string)
}

pub(super) fn control_key(bindings: &[Binding], requested: &str) -> Option<String> {
    (!control_triggers(bindings, requested).is_empty()).then(|| requested.to_string())
}

pub(super) fn control_triggers(bindings: &[Binding], key: &str) -> Vec<(u32, String)> {
    (0..16).filter_map(|bits| {
        let modifiers = [(1, "SHIFT"), (4, "CTRL"), (8, "ALT"), (64, "SUPER")];
        let mask = modifiers.iter().enumerate().fold(0, |mask, (index, (bit, _))| {
            if bits & (1 << index) != 0 { mask | bit } else { mask }
        });
        let mut parts: Vec<&str> = modifiers.iter().filter(|(bit, _)| mask & bit != 0).map(|(_, name)| *name).collect();
        parts.push(key);
        let trigger = parts.join(" + ");
        free(bindings, std::slice::from_ref(&trigger), false).then_some((mask, trigger))
    }).collect()
}

pub(super) fn status(id: &str, requested: String, active: Option<String>, codes: Vec<String>) {
    let conflict = if matches!(id, "cancel" | "handsfree") {
        bindings(id).ok().and_then(|bindings| control_conflict(id, &requested, &bindings, &super::configured_keys(&super::EFFECTIVE_KEYS)))
    } else { None };
    let note = match active.as_deref() {
        Some(active) if active != requested => Some(format!(
            "{requested} is already assigned on this desktop. Using {active}."
        )),
        None => Some(
            "No available shortcut. Change the desktop binding or choose another shortcut.".into(),
        ),
        _ => None,
    };
    super::super::shortcut_status::publish(super::super::shortcut_status::ShortcutStatus {
        id: id.into(),
        requested,
        active,
        codes,
        note,
        conflict,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handsfree_reports_only_the_desktop_chord_used_while_holding_dictation() {
        let bindings = vec![Binding { modmask: 68, key: "SPACE".into(), description: "Background switcher".into(), ..Binding::default() }];
        assert!(control_key(&bindings, "Space").is_some());
        let conflict = control_conflict("handsfree", "Space", &bindings, &[super::super::CTRL, super::super::SUPER]).unwrap();
        assert_eq!(conflict.trigger, "CTRL + SUPER + Space");
        assert_eq!(conflict.description.as_deref(), Some("Background switcher"));
        assert!(control_conflict("handsfree", "Space", &bindings, &[super::super::ALT]).is_none());
        assert!(control_conflict("cancel", "Space", &bindings, &[super::super::CTRL, super::super::SUPER]).unwrap().while_holding);
    }

    #[test]
    fn owned_binding_names_require_an_exact_name_or_annotation() {
        assert!(owned("Verenu dictation"));
        assert!(owned("Verenu dictation (release)"));
        assert!(owned("Verenu copy last dictation (fallback)"));
        assert!(!owned("Verenu dictation-other"));
        assert!(!owned("Verenu dictation(release)"));
        assert!(!owned("Other Verenu dictation"));
    }

    #[test]
    fn unavailable_controls_do_not_fall_back_to_free_function_keys() {
        let bindings = [Binding { key: "Escape".into(), ..Binding::default() },
            Binding { key: "Space".into(), modmask: 64, ..Binding::default() }];
        assert_eq!(control_key(&bindings, "Escape"), Some("Escape".into()));
        assert_eq!(control_key(&bindings, "Space"), Some("Space".into()));
        assert_eq!(control_key(&[], "Escape"), Some("Escape".into()));
        assert_eq!(control_key(&[], "Space"), Some("Space".into()));
    }

    #[test]
    fn secondary_controls_can_choose_another_key_and_recover_the_requested_key() {
        let bindings = [Binding { key: "Space".into(), modmask: 64, ..Binding::default() }];
        assert_eq!(control_key(&bindings, "Space"), Some("Space".into()));
        assert_eq!(control_key(&bindings, "F8"), Some("F8".into()));
        assert_eq!(control_key(&[], "Space"), Some("Space".into()));
        let another_control = [Binding { key: "F8".into(), ignore_mods: true, ..Binding::default() }];
        assert_eq!(control_key(&another_control, "F8"), None);
    }

    #[test]
    fn controls_leave_occupied_modifier_combinations_to_the_desktop() {
        let bindings = [Binding { key: "Space".into(), modmask: 64, ..Binding::default() }];
        let triggers = control_triggers(&bindings, "Space");
        assert!(triggers.iter().any(|(mask, _)| *mask == 0));
        assert!(triggers.iter().any(|(mask, _)| *mask == 68));
        assert!(!triggers.iter().any(|(mask, _)| *mask == 64));
    }

    #[test]
    fn bare_space_conflict_does_not_disable_free_held_combination() {
        let bindings = [Binding { key: "Space".into(), ..Binding::default() }];
        assert_eq!(control_key(&bindings, "Space").as_deref(), Some("Space"));
        assert!(control_triggers(&bindings, "Space").iter().any(|(mask, _)| *mask == 68));
        assert!(!control_triggers(&bindings, "Space").iter().any(|(mask, _)| *mask == 0));
        assert!(control_conflict("handsfree", "Space", &bindings, &[super::super::CTRL, super::super::SUPER]).is_none());
    }

    #[test]
    fn unknown_desktop_bindings_block_every_control_combination() {
        let bindings = [Binding { catch_all: true, ignore_mods: true, ..Binding::default() }];
        assert!(control_triggers(&bindings, "Space").is_empty());
    }

    #[test]
    fn newly_supported_regular_keys_check_numeric_desktop_bindings() {
        let bindings = [Binding { modmask: 12, keycode: 111, ..Binding::default() }];
        assert!(!free(&bindings, &["CTRL+ALT+Up".into()], false));
        assert!(free(&bindings, &["CTRL+ALT+Down".into()], false));
    }
    #[test]
    fn keysym_bindings_accept_negative_keycodes_and_plus_keys() {
        let bindings: Vec<Binding> = serde_json::from_str(r#"[{"modmask":4,"key":"C","keycode":-1}]"#).unwrap();
        assert!(!free(&bindings, &["Ctrl+C".into()], false));
        assert!(free(&bindings, &["Ctrl+S".into()], false));
        assert_eq!(parse("Ctrl++"), (4, "+".into()));
    }
    #[test]
    fn collision_uses_masks_keycodes_submaps_and_ignores_own_bindings() {
        let occupied = vec![Binding {
            modmask: 12,
            keycode: 54,
            ..Binding::default()
        }];
        assert_eq!(
            choose(&occupied, "Ctrl+Alt+C", &["Ctrl+Alt+F6"], false).as_deref(),
            Some("Ctrl+Alt+F6")
        );
        assert!(free(&occupied, &["Ctrl+C".into()], false));
        assert!(!free(&occupied, &["C".into()], true));
        let own = vec![Binding {
            modmask: 12,
            key: "C".into(),
            description: "Verenu copy last dictation".into(),
            ..Binding::default()
        }];
        assert!(free(&own, &["Ctrl+Alt+C".into()], false));
        assert_eq!(choose(&occupied, "Ctrl+Alt+C", &[], false), None);
    }
    #[test]
    fn either_modifier_order_or_side_can_block_the_whole_chord() {
        let occupied = vec![Binding {
            modmask: 64,
            key: "Control_R".into(),
            ..Binding::default()
        }];
        assert!(!free(
            &occupied,
            &["CTRL + Super_L".into(), "SUPER + Control_R".into()],
            false
        ));
    }

    #[test]
    fn conditional_controls_do_not_steal_modified_keys_and_exhaustion_disables_them() {
        let occupied = vec![Binding {
            modmask: 64,
            key: "SPACE".into(),
            ..Binding::default()
        }];
        assert_eq!(
            choose(&occupied, "Space", &["F9"], true).as_deref(),
            Some("F9")
        );
        let occupied = vec![Binding {
            catch_all: true,
            ..Binding::default()
        }];
        assert_eq!(
            choose(&occupied, "Escape", &["F8", "F9", "F10"], true),
            None
        );
    }
}
