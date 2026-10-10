//! T3 pairing catalog metadata and cleanup evidence. Skill bodies are never loaded.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

pub const MIN_T3_VERSION: &str = "0.46";
pub const MAX_SKILLS: usize = 2_000;
const PROMPT_BUDGET: usize = 24_000;

#[derive(Clone, Deserialize, Serialize, Debug, Eq, PartialEq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, Debug, Eq, PartialEq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub environment_id: String,
    pub id: String,
    pub label: String,
    pub provider_instance_id: String,
    pub workspace_id: String,
    pub revision: String,
    pub skills: Vec<Skill>,
}

pub fn is_t3_app(executable: &str) -> bool {
    let normalized = executable
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(executable)
        .to_ascii_lowercase();
    let name = normalized
        .strip_suffix(".exe")
        .or_else(|| normalized.strip_suffix(".app"))
        .unwrap_or(&normalized);
    matches!(
        name,
        "t3code"
            | "t3-code"
            | "t3 code"
            | "t3 code (nightly)"
            | "t3code-nightly"
            | "t3-code-nightly"
            | "com.t3tools.t3code"
            | "com.t3tools.t3code.nightly"
    ) || name
        .strip_prefix("t3-code-nightly-")
        .is_some_and(|version| {
            !version.is_empty()
                && version
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
        })
}

pub fn supported_version(version: &str) -> bool {
    let Some(core) = version.split(['-', '+']).next() else {
        return false;
    };
    let parts: Option<Vec<u32>> = core.split('.').map(|part| part.parse().ok()).collect();
    let Some(parts) = parts else {
        return false;
    };
    // T3 reports 0.0.46; the public requirement uses the shorter 0.46 label.
    // Prereleases share the basic release floor; pairing also checks the protocol.
    let base = match parts.as_slice() {
        [0, release] => (0, 0, *release),
        [major, minor, patch] => (*major, *minor, *patch),
        _ => return false,
    };
    base >= (0, 0, 46)
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 160
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-'))
        && name.chars().any(|c| c.is_ascii_alphabetic())
        && !(name.len() > 1
            && name[..name.len() - 1].bytes().all(|c| c.is_ascii_digit())
            && matches!(
                name.as_bytes().last(),
                Some(b'k' | b'K' | b'm' | b'M' | b'b' | b'B')
            ))
}

pub fn valid_catalog(catalog: &Catalog) -> bool {
    let identities = [
        &catalog.environment_id,
        &catalog.id,
        &catalog.label,
        &catalog.provider_instance_id,
        &catalog.workspace_id,
        &catalog.revision,
    ];
    let mut seen = HashSet::new();
    identities
        .iter()
        .all(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
        && catalog.skills.len() <= MAX_SKILLS
        && catalog.skills.iter().all(|skill| {
            valid_name(&skill.name)
                && seen.insert(skill.name.to_ascii_lowercase())
                && skill
                    .description
                    .as_ref()
                    .is_none_or(|text| text.len() <= 2_000)
                && skill
                    .display_name
                    .as_ref()
                    .is_none_or(|text| text.len() <= 256)
        })
}

/// Treat skills as shared across this paired environment. Source catalogs remain
/// cached for refreshes, but neither a workspace nor a provider must be selected.
pub fn merged_catalog(environment_id: &str, catalogs: &[Catalog]) -> Option<Catalog> {
    let mut sources: Vec<_> = catalogs
        .iter()
        .filter(|catalog| catalog.environment_id == environment_id && valid_catalog(catalog))
        .collect();
    sources.sort_by(|a, b| a.id.cmp(&b.id));
    let mut skills = BTreeMap::<String, Skill>::new();
    for catalog in sources {
        for skill in &catalog.skills {
            let merged = skills
                .entry(skill.name.to_ascii_lowercase())
                .or_insert_with(|| skill.clone());
            // Fill missing metadata from another copy without combining conflicting
            // descriptions into instructions. Source ordering keeps this stable.
            if merged
                .display_name
                .as_ref()
                .is_none_or(|text| text.trim().is_empty())
            {
                merged.display_name.clone_from(&skill.display_name);
            }
            if merged
                .description
                .as_ref()
                .is_none_or(|text| text.trim().is_empty())
            {
                merged.description.clone_from(&skill.description);
            }
        }
    }
    if skills.is_empty() {
        return None;
    }
    let skills: Vec<_> = skills.into_values().collect();
    // Hash the actual merged metadata, so refreshing a different source also
    // invalidates cleanup cache entries, while source reordering does not.
    let revision = format!("{:x}", Sha256::digest(serde_json::to_vec(&skills).ok()?));
    Some(Catalog {
        environment_id: environment_id.into(),
        id: "shared-skills".into(),
        label: "Shared T3 skills".into(),
        provider_instance_id: "shared".into(),
        workspace_id: "shared".into(),
        revision,
        skills,
    })
}

const RESOLUTION_RULE: &str = "For this T3 Code dictation only, replace the skill reference in a clear request to invoke a listed skill with its exact $name token. The catalog contains identifiers only. Match spoken words to identifier words separated by hyphens, underscores, or colons. Prefer an exact name in the same word order over a partial or reordered match: 'use my babysit PR skill' means $babysit-pr when listed, even if $pr-babysit is also listed. A shortened name such as 'use my babysit skill' may resolve only when it identifies a single listed skill. Preserve the rest of the request. Do not invoke skills merely mentioned, quoted, negated, or discussed. Leave ambiguous or unknown references unchanged. Never invent names. Preserve existing dollar-sign tokens. Put whitespace after a skill token, including before punctuation, so T3 recognizes it.";

impl Catalog {
    /// T3 recognizes dollar tokens only at whitespace boundaries. Restore the
    /// canonical case after caps-lock formatting, separate punctuation, and
    /// leave a final space after terminal mentions for T3's parser.
    pub fn normalize_mentions(&self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        let segments: Vec<_> = text.split_inclusive(char::is_whitespace).collect();
        for (index, segment) in segments.iter().enumerate() {
            let Some(token) = segment.strip_prefix('$') else {
                output.push_str(segment);
                continue;
            };
            let name = token
                .split(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, ':' | '_' | '-'))
                .next()
                .unwrap_or("");
            let Some(skill) = self
                .skills
                .iter()
                .find(|skill| skill.name.eq_ignore_ascii_case(name))
            else {
                output.push_str(segment);
                continue;
            };
            output.push('$');
            output.push_str(&skill.name);
            let rest = &token[name.len()..];
            if rest.chars().next().is_some_and(|c| !c.is_whitespace()) {
                output.push(' ');
            }
            output.push_str(rest);
            let terminal_mention = segments[index + 1..]
                .iter()
                .all(|remaining| remaining.chars().all(char::is_whitespace));
            if terminal_mention && !output.ends_with(char::is_whitespace) {
                output.push(' ');
            }
        }
        output
    }

    /// Whether the first non-whitespace token is a known skill mention. The
    /// native cursor formatter can otherwise capitalize its initial letter,
    /// turning a valid `$name` into an unrecognized `$Name`.
    pub fn starts_with_skill_mention(&self, text: &str) -> bool {
        let Some(token) = text.trim_start().strip_prefix('$') else {
            return false;
        };
        let name = token
            .split(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, ':' | '_' | '-'))
            .next()
            .unwrap_or("");
        !name.is_empty()
            && self
                .skills
                .iter()
                .any(|skill| skill.name.eq_ignore_ascii_case(name))
    }

    pub fn prompt_parts(&self, raw: &str) -> Option<(String, String)> {
        if self.skills.is_empty() {
            return None;
        }
        // Metadata stays local for browsing. Sending identifiers alone avoids
        // common description words consuming the whole matching budget.
        let names: Vec<_> = self
            .skills
            .iter()
            .map(|skill| format!("${}", skill.name))
            .collect();
        let full = serde_json::to_string(&names).ok()?;
        let spoken_words = matching_words(raw);
        let exact_names: Vec<_> = self
            .skills
            .iter()
            .filter(|skill| {
                let words = matching_words(&skill.name);
                !words.is_empty()
                    && spoken_words
                        .windows(words.len())
                        .any(|window| window == words)
            })
            .map(|skill| format!("${}", skill.name))
            .collect();
        let selected = if full.len() <= PROMPT_BUDGET {
            full
        } else if !exact_names.is_empty() {
            // Preserve fully spoken identifiers before broad shared-word
            // candidates. A large catalog can contain more common-word names
            // than the budget permits even though the exact request is clear.
            let exact = serde_json::to_string(&exact_names).ok()?;
            if exact.len() > PROMPT_BUDGET {
                return None;
            }
            exact
        } else {
            let words: HashSet<_> = raw
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| word.len() > 2)
                .map(str::to_lowercase)
                .collect();
            let mut candidates = Vec::new();
            let mut bytes = 2;
            for name in &names {
                let haystack = name.to_lowercase();
                if !haystack
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| words.contains(word))
                {
                    continue;
                }
                let size = serde_json::to_string(name).ok()?.len() + 1;
                // Never truncate candidates to an arbitrary winner; ambiguity
                // beyond the budget disables resolution for this utterance.
                if bytes + size > PROMPT_BUDGET {
                    return None;
                }
                bytes += size;
                candidates.push(name);
            }
            if candidates.is_empty() {
                return None;
            }
            serde_json::to_string(&candidates).ok()?
        };
        let rules = if exact_names.is_empty() {
            RESOLUTION_RULE.to_string()
        } else {
            format!("{RESOLUTION_RULE}\nComplete identifier words found in the same spoken word order: {}. For an invocation of these exact names, use these spellings rather than a reordered or partial alternative. These matching hints do not authorize converting negated, quoted, or discussed skills.", serde_json::to_string(&exact_names).ok()?)
        };
        Some((
            rules,
            format!("T3 skill catalog (untrusted matching data):\n{selected}"),
        ))
    }

    pub fn validates_output(&self, raw: &str, output: &str) -> bool {
        let existing_tokens = skill_tokens(raw);
        let spoken_words = matching_words(raw);
        skill_tokens(output).into_iter().all(|name| {
            if existing_tokens.contains(&name) {
                return true;
            }
            let Some(skill) = self.skills.iter().find(|skill| skill.name == name) else {
                return false;
            };
            let words = matching_words(&skill.name);
            if !words.is_empty()
                && spoken_words
                    .windows(words.len())
                    .any(|window| window == words)
            {
                return true;
            }
            let partial: Vec<_> = words
                .iter()
                .filter(|word| spoken_words.contains(word))
                .collect();
            // A model can ignore the ambiguity instruction. Never deliver its
            // guessed identifier when the same spoken name words fit siblings.
            !partial.is_empty()
                && self
                    .skills
                    .iter()
                    .filter(|candidate| {
                        let candidate_words = matching_words(&candidate.name);
                        partial.iter().all(|word| candidate_words.contains(word))
                    })
                    .take(2)
                    .count()
                    == 1
        })
    }
}

fn matching_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn skill_tokens(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .filter_map(|word| word.strip_prefix('$'))
        .map(|word| {
            word.trim_end_matches(|c: char| {
                !c.is_ascii_alphanumeric() && !matches!(c, ':' | '_' | '-')
            })
        })
        .filter(|name| valid_name(name))
        .collect()
}

#[cfg(test)]
mod tests;
