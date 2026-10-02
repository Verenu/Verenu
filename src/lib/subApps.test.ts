import { describe, expect, it } from 'vitest';
import { suggestLabel, titleRuleMatches } from './subApps';

describe('sub-app title rules', () => {
  it('matches case and whitespace insensitively like the Rust resolver', () => {
    expect(titleRuleMatches('verenu  discord', 'contains', '#general - Verenu Discord')).toBe(true);
    expect(titleRuleMatches('#general', 'starts_with', '#General   - Verenu')).toBe(true);
    expect(titleRuleMatches('#general', 'equals', '#general - Verenu')).toBe(false);
    expect(titleRuleMatches('   ', 'contains', 'anything')).toBe(false);
  });

  it('suggests the first title segment as the name', () => {
    expect(suggestLabel('#design | Acme')).toBe('#design');
    expect(suggestLabel('main.rs - verenu')).toBe('main.rs');
    expect(suggestLabel('Inbox')).toBe('Inbox');
  });
});

import { chordFromKeyboardEvent, subAppCaptureKeys } from './subApps';

describe('sub-app capture chord', () => {
  const press = (init: Partial<KeyboardEvent>) => ({ ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...init }) as KeyboardEvent;

  it('records modifiers in canonical order and rejects typing chords', () => {
    expect(chordFromKeyboardEvent(press({ code: 'KeyS', ctrlKey: true, altKey: true, shiftKey: true }))).toBe('Ctrl+Alt+Shift+S');
    expect(chordFromKeyboardEvent(press({ code: 'F5', metaKey: true }))).toBe('Super+F5');
    expect(chordFromKeyboardEvent(press({ code: 'KeyS', shiftKey: true }))).toBeNull();
    expect(chordFromKeyboardEvent(press({ code: 'ControlLeft', ctrlKey: true }))).toBeNull();
  });

  it('splits a stored chord into display keys', () => {
    expect(subAppCaptureKeys('Ctrl+Alt+Shift+S')).toEqual(['Ctrl', 'Alt', 'Shift', 'S']);
  });
});
