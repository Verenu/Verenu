import { describe, expect, it } from 'vitest';
import { matchesAppSearch, rankAppMatches } from './helpers';

const apps = [
  { name: 'Chrome', exe: 'com.android.chrome' },
  { name: 'Google Maps', exe: 'com.google.android.apps.maps' },
  { name: 'Maps.me', exe: 'com.mapswithme.maps.pro' },
  { name: 'Slack', exe: 'com.slack' },
  { name: 'Gmail', exe: 'com.google.android.gm' },
];

describe('rankAppMatches', () => {
  it('keeps the list for an empty query', () => {
    expect(rankAppMatches(apps, '  ')).toEqual(apps);
  });

  it('puts names that start with the query before later matches', () => {
    expect(rankAppMatches(apps, 'maps').map((app) => app.name)).toEqual(['Maps.me', 'Google Maps']);
  });

  it('ranks a word prefix above a package-only match', () => {
    const names = rankAppMatches(apps, 'google').map((app) => app.name);
    expect(names[0]).toBe('Google Maps');
    expect(names).toContain('Gmail');
  });

  it('drops apps that do not match', () => {
    expect(rankAppMatches(apps, 'zzz')).toEqual([]);
    expect(rankAppMatches(apps, 'slack').every((app) => matchesAppSearch(app, 'slack'))).toBe(true);
  });

  it('searches only the visible name when asked to (Android package names)', () => {
    expect(rankAppMatches(apps, 'com', false).length).toBe(apps.length);
    expect(rankAppMatches(apps, 'com', true)).toEqual([]);
    expect(rankAppMatches(apps, 'chro', true).map((app) => app.name)).toEqual(['Chrome']);
  });
});
