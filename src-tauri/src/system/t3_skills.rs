//! T3 pairing catalog metadata and cleanup evidence. Skill bodies are never loaded.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};

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
        } else {
            let exact: HashSet<_> = exact_names.iter().map(String::as_str).collect();
            let unique_partial = self.unique_partial_matches(&spoken_words);
            let candidates: Vec<_> = self
                .skills
                .iter()
                .enumerate()
                .filter(|(index, skill)| {
                    let name = format!("${}", skill.name);
                    exact.contains(name.as_str()) || unique_partial[*index]
                })
                .map(|(_, skill)| format!("${}", skill.name))
                .collect();
            if candidates.is_empty() {
                return None;
            }
            let selected = serde_json::to_string(&candidates).ok()?;
            // Keep every exact and uniquely resolvable partial name; never
            // truncate to an arbitrary winner if those requests exceed budget.
            if selected.len() > PROMPT_BUDGET {
                return None;
            }
            selected
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
            let Some(skill) = self
                .skills
                .iter()
                .find(|skill| skill.name.eq_ignore_ascii_case(name))
            else {
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
            // A model can ignore the ambiguity instruction. Never deliver its
            // guessed identifier when the same spoken name words fit siblings.
            self.has_unique_partial_match(&skill.name, &spoken_words)
        })
    }

    fn has_unique_partial_match(&self, name: &str, spoken_words: &[String]) -> bool {
        let words = matching_words(name);
        let partial: Vec<_> = words
            .iter()
            .filter(|word| spoken_words.contains(word))
            .collect();
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
    }

    fn unique_partial_matches(&self, spoken_words: &[String]) -> Vec<bool> {
        let spoken: HashSet<_> = spoken_words.iter().map(String::as_str).collect();
        let skill_words: Vec<_> = self
            .skills
            .iter()
            .map(|skill| matching_words(&skill.name))
            .collect();
        let mut postings = HashMap::<String, Vec<usize>>::new();
        for (index, words) in skill_words.iter().enumerate() {
            let mut seen = HashSet::new();
            for word in words {
                if seen.insert(word.as_str()) {
                    postings.entry(word.clone()).or_default().push(index);
                }
            }
        }

        skill_words
            .iter()
            .map(|words| {
                let partial: Vec<_> = words
                    .iter()
                    .filter(|word| spoken.contains(word.as_str()))
                    .collect();
                let Some(rarest) = partial
                    .iter()
                    .min_by_key(|word| postings.get(word.as_str()).map_or(0, Vec::len))
                else {
                    return false;
                };
                let Some(candidates) = postings.get(rarest.as_str()) else {
                    return false;
                };
                let mut matches = 0;
                for candidate in candidates {
                    if partial.iter().all(|word| {
                        skill_words[*candidate]
                            .iter()
                            .any(|candidate_word| candidate_word == *word)
                    }) {
                        matches += 1;
                        if matches > 1 {
                            return false;
                        }
                    }
                }
                matches == 1
            })
            .collect()
    }
}

fn matching_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn skill_tokens(text: &str) -> Vec<&str> {
    text.match_indices('$')
        .filter_map(|(index, _)| {
            if index > 0 && text.as_bytes()[index - 1] == b'$' {
                return None;
            }
            let rest = &text[index + 1..];
            let length = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, ':' | '_' | '-'))
                .unwrap_or(rest.len());
            let name = &rest[..length];
            valid_name(name).then_some(name)
        })
        .collect()
}

#[cfg(test)]
mod tests;
