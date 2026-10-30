export function skillToken(name: string): string {
  return `$${name}`;
}

export function sortedSkillNames(skills: readonly { name: string }[]): string[] {
  return [...new Set(skills.map((skill) => skill.name))].sort((a, b) => a.localeCompare(b, 'en'));
}

export function filterSkillNames(names: readonly string[], query: string): string[] {
  const needle = query.trim().replace(/^\$/, '').toLowerCase();
  return needle ? names.filter((name) => name.toLowerCase().includes(needle)) : [...names];
}

export function isSkillExcluded(excluded: readonly string[] | undefined, name: string): boolean {
  const lower = name.toLowerCase();
  return !!excluded?.some((entry) => entry.toLowerCase() === lower);
}

export function timeAgo(epochSeconds: number, nowMs = Date.now()): string {
  const seconds = Math.max(0, Math.round(nowMs / 1000 - epochSeconds));
  if (seconds < 60) return 'just now';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  return `${days} d ago`;
}
