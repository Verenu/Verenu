import { describe, expect, it } from 'vitest';
import { matchesAppSearch, rankAppMatches } from './helpers';

const apps = [
  { name: 'Chrome', exe: 'com.android.chrome' },
  { name: 'Google Maps', exe: 'com.google.android.apps.maps' },
  { name: 'Maps.me', exe: 'com.mapswithme.maps.pro' },
  { name: 'Slack', exe: 'com.slack' },
  { name: 'Gmail', exe: 'com.google.android.gm' },
];

describe('rankAppMatches on Android', () => {
  it('keeps the list for an empty query', () => {
    expect(rankAppMatches(apps, '  ', true)).toEqual(apps);
  });

  it('puts names that start with the query before later matches', () => {
    expect(rankAppMatches(apps, 'maps', true).map((app) => app.name)).toEqual(['Maps.me', 'Google Maps']);
  });

  it('ranks a name-prefix match above a word-prefix match', () => {
    expect(rankAppMatches(apps, 'g', true).map((app) => app.name)).toEqual(['Google Maps', 'Gmail']);
  });

  it('searches only the visible name, never the package', () => {
    expect(rankAppMatches(apps, 'com', true)).toEqual([]);
    expect(rankAppMatches(apps, 'chro', true).map((app) => app.name)).toEqual(['Chrome']);
  });

  it('drops apps that do not match', () => {
    expect(rankAppMatches(apps, 'zzz', true)).toEqual([]);
  });
});

describe('rankAppMatches on desktop', () => {
  it('only filters, in the order the platform listed the apps', () => {
    expect(rankAppMatches(apps, 'maps', false).map((app) => app.name)).toEqual(['Google Maps', 'Maps.me']);
  });

  it('still matches the executable, as before', () => {
    expect(rankAppMatches(apps, 'com.slack', false).map((app) => app.name)).toEqual(['Slack']);
    expect(rankAppMatches(apps, 'slack', false).every((app) => matchesAppSearch(app, 'slack', false))).toBe(true);
  });
});

describe('app search normalization', () => {
  const variants = [
    { name: 'Visual Studio Code', exe: 'Code.exe' },
    { name: 'VS_Code', exe: 'vscode.exe' },
    { name: '', exe: 'VSCode.exe' },
    { name: 'Other', exe: 'visual-studio-code.exe' },
  ];

  it('matches compact names while excluding executables on Android', () => {
    expect(rankAppMatches(variants, '  VSCODE  ', true)).toEqual([variants[2], variants[1]]);
    expect(rankAppMatches(variants, 'VisualStudioCode', false)).toEqual([variants[0], variants[3]]);
  });

  it('retains input order for equally ranked name matches', () => {
    expect(rankAppMatches(variants, 'code', true)).toEqual(variants.slice(0, 3));
  });

  it('handles punctuation-only queries without matching every compact name', () => {
    expect(rankAppMatches(apps, '.', true)).toEqual([apps[2]]);
    expect(rankAppMatches(apps, '.', false)).toEqual(apps);
    expect(matchesAppSearch(apps[0], '  ', true)).toBe(true);
  });
});
