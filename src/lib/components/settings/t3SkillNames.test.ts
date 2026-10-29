import { describe, expect, it } from 'vitest';
import { filterSkillNames, isSkillExcluded, skillToken, sortedSkillNames, timeAgo } from './t3SkillNames';

describe('T3 skill names', () => {
  it('copies the exact $skill-name token', () => {
    expect(skillToken('babysit-pr')).toBe('$babysit-pr');
  });

  it('lists unique names alphabetically and drops everything but the name', () => {
    const skills = [
      { name: 'skill-designer', description: 'Create and improve skills.' },
      { name: 'babysit-pr', description: 'Private description sentinel' },
      { name: 'skill-designer', description: 'Duplicate from another workspace' },
      { name: 'Pr-Review' },
    ];
    expect(sortedSkillNames(skills)).toEqual(['babysit-pr', 'Pr-Review', 'skill-designer']);
  });

  it('filters by name only, ignoring a leading dollar sign and case', () => {
    const names = ['babysit-pr', 'pr-babysit', 'skill-designer'];
    expect(filterSkillNames(names, '$BABYSIT')).toEqual(['babysit-pr', 'pr-babysit']);
    expect(filterSkillNames(names, 'designer')).toEqual(['skill-designer']);
    expect(filterSkillNames(names, '  ')).toEqual(names);
  });

  it('never matches description text because descriptions are not passed in', () => {
    const names = sortedSkillNames([{ name: 'babysit-pr' }]);
    expect(filterSkillNames(names, 'sentinel')).toEqual([]);
  });

  it('matches excluded skills case-insensitively', () => {
    expect(isSkillExcluded(['Babysit-PR'], 'babysit-pr')).toBe(true);
    expect(isSkillExcluded(['file-pr'], 'babysit-pr')).toBe(false);
    expect(isSkillExcluded(undefined, 'babysit-pr')).toBe(false);
  });

  it('describes recent pulls in short relative terms', () => {
    const now = Date.UTC(2026, 0, 1, 12, 0, 0);
    expect(timeAgo(now / 1000 - 20, now)).toBe('just now');
    expect(timeAgo(now / 1000 - 5 * 60, now)).toBe('5 min ago');
    expect(timeAgo(now / 1000 - 3 * 3600, now)).toBe('3 h ago');
    expect(timeAgo(now / 1000 - 2 * 86400, now)).toBe('2 d ago');
  });
});
