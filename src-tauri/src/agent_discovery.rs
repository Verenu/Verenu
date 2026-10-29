use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use sha2::{Digest, Sha256};

const GUIDE: &str = include_str!("../../Agent-Skills/verenu-install/SKILL.md");
// The first discovery build recorded only "registered", without a content hash.
const FIRST_GUIDE_HASH: &str = "79f2dca761341b7553a55b9d10eb3565e50e694e538e26662d66a206c3843f61";

fn digest(contents: &[u8]) -> String {
    format!("{:x}", Sha256::digest(contents))
}

const HARNESSES: [(&str, &str, &str); 5] = [
    ("codex", ".codex", ".agents/skills"),
    ("claude", ".claude", ".claude/skills"),
    ("cursor", ".cursor", ".cursor/skills"),
    ("grok", ".grok", ".grok/skills"),
    ("opencode", ".config/opencode", ".config/opencode/skills"),
];

pub(crate) fn register(home: &Path, data: &Path) -> io::Result<()> {
    let state = data.join("agent-discovery");
    fs::create_dir_all(&state)?;
    for (id, config, skills) in HARNESSES {
        let marker = state.join(id);
        if !home.join(config).is_dir() {
            continue;
        }
        let destination = home.join(skills).join("verenu-install");
        let file = destination.join("SKILL.md");
        if marker.exists() {
            // Update only a guide that is still byte-for-byte the version we
            // installed. A removed or edited skill remains the user's choice.
            let previous = fs::read_to_string(&marker)?;
            if let Ok(current) = fs::read(&file) {
                let current_hash = digest(&current);
                let owned = previous.trim() == current_hash
                    || (previous.trim() == "registered" && current_hash == FIRST_GUIDE_HASH);
                if owned && current != GUIDE.as_bytes() {
                    fs::write(&file, GUIDE)?;
                    fs::write(&marker, digest(GUIDE.as_bytes()))?;
                }
            }
            continue;
        }
        fs::create_dir_all(&destination)?;
        match OpenOptions::new().write(true).create_new(true).open(&file) {
            Ok(mut output) => {
                if let Err(error) = output.write_all(GUIDE.as_bytes()) {
                    drop(output);
                    let _ = fs::remove_file(file);
                    return Err(error);
                }
                fs::write(&marker, digest(GUIDE.as_bytes()))?;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                fs::write(&marker, b"external\n")?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_only_detected_harnesses_without_replacing_or_reinstalling() {
        let root = std::env::temp_dir().join(format!("verenu-discovery-{}", uuid::Uuid::new_v4()));
        let home = root.join("home");
        let data = root.join("data");
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::create_dir_all(home.join(".claude/skills/verenu-install")).unwrap();
        let existing = home.join(".claude/skills/verenu-install/SKILL.md");
        fs::write(&existing, "user skill").unwrap();
        register(&home, &data).unwrap();
        let codex = home.join(".agents/skills/verenu-install/SKILL.md");
        assert_eq!(fs::read_to_string(&codex).unwrap(), GUIDE);
        assert_eq!(fs::read_to_string(existing).unwrap(), "user skill");
        fs::write(&codex, "previous shipped guide").unwrap();
        fs::write(data.join("agent-discovery/codex"), digest(b"previous shipped guide")).unwrap();
        register(&home, &data).unwrap();
        assert_eq!(fs::read_to_string(&codex).unwrap(), GUIDE);
        fs::write(&codex, "customized guide").unwrap();
        register(&home, &data).unwrap();
        assert_eq!(fs::read_to_string(&codex).unwrap(), "customized guide");
        assert!(!home.join(".grok/skills/verenu-install").exists());
        fs::remove_file(&codex).unwrap();
        register(&home, &data).unwrap();
        assert!(!codex.exists());
        fs::create_dir_all(home.join(".grok")).unwrap();
        register(&home, &data).unwrap();
        assert!(home.join(".grok/skills/verenu-install/SKILL.md").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
