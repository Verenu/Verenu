/** The exact mention T3 Code receives for a shared skill. */
export function skillToken(name: string): string {
  return `$${name}`;
}

/** Unique skill names in alphabetical order. Descriptions are never shown or searched. */
export function sortedSkillNames(skills: readonly { name: string }[]): string[] {
  return [...new Set(skills.map((skill) => skill.name))].sort((a, b) => a.localeCompare(b, 'en'));
}

/** Filters names only. A leading `$` in the query is ignored so `$babysit` finds `babysit-pr`. */
export function filterSkillNames(names: readonly string[], query: string): string[] {
  const needle = query.trim().replace(/^\$/, '').toLowerCase();
  return needle ? names.filter((name) => name.toLowerCase().includes(needle)) : [...names];
}
