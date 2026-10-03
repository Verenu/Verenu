import { describe, expect, it } from 'vitest';
import { encodeContextIcon, iconKindFor, iconText, parseContextIcon } from './contextIcon';

const colors = { background: '#123456', foreground: '#abcdef' };
const roundTrip = (text: string) => parseContextIcon(encodeContextIcon({ text, ...colors }));

describe('context icons', () => {
  it('round trips letter artwork through the existing storage field', () => {
    expect(roundTrip('WE')).toEqual({ kind: 'letters', text: 'WE', ...colors });
  });
  it('limits input to two visible characters without splitting clusters', () => {
    expect(iconText('👩🏽‍💻🌈🎉')).toBe('👩🏽‍💻🌈');
    expect(iconText('🇨🇦')).toBe('🇨🇦');
    expect(iconText('1️⃣2️⃣3')).toBe('1️⃣2️⃣');
    expect(iconText(' éAB ')).toBe('éA');
    expect(iconText('A 🙂 B')).toBe('A🙂');
  });
  it('infers presentation from content', () => {
    expect(iconKindFor('WE')).toBe('letters');
    expect(iconKindFor('👩🏽‍💻')).toBe('emoji');
    expect(iconKindFor('🌈🚀')).toBe('emoji');
    expect(iconKindFor('🇨🇦')).toBe('emoji');
    expect(iconKindFor('1️⃣')).toBe('emoji');
    expect(iconKindFor('A🚀')).toBe('letters');
    expect(iconKindFor('1')).toBe('letters');
    expect(iconKindFor('')).toBe('letters');
  });
  it('saves and reopens a complex emoji, two emoji, and a mixed pair', () => {
    expect(roundTrip('👩🏽‍💻')?.kind).toBe('emoji');
    expect(roundTrip('🌈🚀')).toMatchObject({ kind: 'emoji', text: '🌈🚀' });
    expect(roundTrip('A🚀')).toMatchObject({ kind: 'letters', text: 'A🚀', ...colors });
    expect(roundTrip('éA')?.text).toBe('éA');
  });
  it('keeps legacy stored emoji and letters parseable', () => {
    const legacy = (kind: string, text: string) => 'custom-icon:' + JSON.stringify({ kind, text, background: '#164e63', foreground: '#38bdf8' });
    expect(parseContextIcon(legacy('emoji', '🌈'))).toMatchObject({ kind: 'emoji', text: '🌈', background: '#164e63' });
    expect(parseContextIcon(legacy('letters', 'WE'))).toMatchObject({ kind: 'letters', text: 'WE' });
  });
  it('rejects malformed artwork and unsafe colors, and empty input encodes to nothing', () => {
    for (const value of [null, 'code', 'custom-icon:{', 'custom-icon:' + JSON.stringify({ kind: 'letters', text: 'ABC', ...colors }), 'custom-icon:' + JSON.stringify({ kind: 'emoji', text: '🌈', background: 'url(example)', foreground: '#ffffff' })]) {
      expect(parseContextIcon(value)).toBeNull();
    }
    expect(encodeContextIcon({ text: ' ', ...colors })).toBeNull();
  });
});
