import { describe, expect, it } from 'vitest';
import audioSource from '../../src-tauri/src/media/audio.rs?raw';
import androidSource from '../../src-tauri/src/android/mod.rs?raw';
import { ENVELOPE_WINDOW_MS } from './pillVisualizer';
import {
  ANDROID_PILL_POSITION_OPTIONS,
  ANDROID_PILL_SCREEN_POSITION_OPTIONS,
} from './settings';

describe('frontend/Rust constant alignment', () => {
  it('uses the backend audio envelope sample interval', () => {
    const declaration = audioSource.match(/pub const ENVELOPE_WINDOW_MS:\s*u32\s*=\s*(\d+)\s*;/);
    expect(declaration, 'Rust ENVELOPE_WINDOW_MS declaration').not.toBeNull();
    expect(ENVELOPE_WINDOW_MS).toBe(Number(declaration?.[1]));
  });

  it.each([
    ['ANDROID_PILL_SCREEN_POSITIONS', ANDROID_PILL_SCREEN_POSITION_OPTIONS],
    ['ANDROID_PILL_POSITIONS', ANDROID_PILL_POSITION_OPTIONS],
  ])('offers exactly the positions accepted by Rust %s', (name, options) => {
    const declaration = androidSource.match(
      new RegExp(`pub const ${name}:\\s*\\[&str;\\s*\\d+\\]\\s*=\\s*\\[([^\\]]+)\\];`),
    );
    expect(declaration, `Rust ${name} declaration`).not.toBeNull();
    const positions = [...(declaration?.[1] ?? '').matchAll(/"([^"]+)"/g)].map((match) => match[1]);
    expect(positions.length).toBeGreaterThan(0);
    expect(options.map(({ id }) => id).sort()).toEqual(positions.sort());
  });
});
