//! The sub-app capture shortcut: one or more modifiers plus a letter, digit,
//! or function key, stored as a canonical string such as `Ctrl+Alt+Shift+S`.
//! `Super` is the Windows/Command key.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordKey {
    Letter(char),
    Digit(u8),
    Function(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
    pub key: ChordKey,
}

impl Chord {
    /// Ctrl+Alt+Shift+S (Cmd+Option+Shift+S on macOS). Plain Ctrl+Alt+S is
    /// JetBrains' Settings shortcut, which a global hook would steal.
    pub fn default_for_platform() -> Self {
        Self {
            ctrl: !cfg!(target_os = "macos"),
            alt: true,
            shift: true,
            super_key: cfg!(target_os = "macos"),
            key: ChordKey::Letter('S'),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let mut chord = Self {
            ctrl: false,
            alt: false,
            shift: false,
            super_key: false,
            key: ChordKey::Letter('S'),
        };
        let mut key = None;
        for part in value.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => chord.ctrl = true,
                "alt" | "option" => chord.alt = true,
                "shift" => chord.shift = true,
                "super" | "cmd" | "meta" | "win" => chord.super_key = true,
                other => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(parse_key(other)?);
                }
            }
        }
        chord.key = key?;
        // A bare key or Shift+key would hijack ordinary typing.
        (chord.ctrl || chord.alt || chord.super_key).then_some(chord)
    }

    /// Hyprland `hl.bind` form, e.g. `CTRL + ALT + SHIFT + S`.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn hyprland(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("CTRL".to_string());
        }
        if self.alt {
            parts.push("ALT".to_string());
        }
        if self.shift {
            parts.push("SHIFT".to_string());
        }
        if self.super_key {
            parts.push("SUPER".to_string());
        }
        parts.push(self.key_name());
        parts.join(" + ")
    }

    pub fn key_name(&self) -> String {
        match self.key {
            ChordKey::Letter(c) => c.to_string(),
            ChordKey::Digit(d) => d.to_string(),
            ChordKey::Function(n) => format!("F{n}"),
        }
    }

    /// Windows virtual-key code for the non-modifier key.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn windows_vk(&self) -> u32 {
        match self.key {
            ChordKey::Letter(c) => c as u32,
            ChordKey::Digit(d) => 0x30 + u32::from(d),
            ChordKey::Function(n) => 0x6F + u32::from(n),
        }
    }

    /// W3C `KeyboardEvent.code` name, as used by `global_hotkey` on macOS.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn web_code(&self) -> String {
        match self.key {
            ChordKey::Letter(c) => format!("Key{c}"),
            ChordKey::Digit(d) => format!("Digit{d}"),
            ChordKey::Function(n) => format!("F{n}"),
        }
    }
}

fn parse_key(value: &str) -> Option<ChordKey> {
    let upper = value.to_ascii_uppercase();
    if upper.len() == 1 {
        let c = upper.chars().next()?;
        if c.is_ascii_uppercase() {
            return Some(ChordKey::Letter(c));
        }
        if c.is_ascii_digit() {
            return Some(ChordKey::Digit(c as u8 - b'0'));
        }
        return None;
    }
    let n: u8 = upper.strip_prefix('F')?.parse().ok()?;
    (1..=12).contains(&n).then_some(ChordKey::Function(n))
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        if self.super_key {
            parts.push("Super".into());
        }
        parts.push(self.key_name());
        f.write_str(&parts.join("+"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_canonically() {
        let chord = Chord::parse("shift + ctrl+alt+s").unwrap();
        assert_eq!(chord.to_string(), "Ctrl+Alt+Shift+S");
        assert_eq!(chord.hyprland(), "CTRL + ALT + SHIFT + S");
        assert_eq!(chord.windows_vk(), 0x53);
        assert_eq!(Chord::parse("Cmd+Option+F5").unwrap().to_string(), "Alt+Super+F5");
        assert_eq!(Chord::parse("Ctrl+7").unwrap().web_code(), "Digit7");
    }

    #[test]
    fn rejects_chords_that_would_hijack_typing() {
        for bad in ["S", "Shift+S", "Ctrl+Alt", "Ctrl+S+T", "Ctrl+F13", "Ctrl+Space"] {
            assert!(Chord::parse(bad).is_none(), "{bad}");
        }
    }
}
