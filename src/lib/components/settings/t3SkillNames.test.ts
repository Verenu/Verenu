import { describe, expect, it } from 'vitest';
import { filterSkillNames, skillToken, sortedSkillNames } from './t3SkillNames';

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
});
