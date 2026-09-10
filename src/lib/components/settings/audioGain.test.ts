import { describe, expect, it } from 'vitest';
import { shouldPersistMicGain } from './audioGain';

describe('mic gain persistence', () => {
  it('does not save the default when settings loading never established a value', () => {
    expect(shouldPersistMicGain(3.5, null, false)).toBe(false);
  });

  it('saves an explicit slider change', () => {
    expect(shouldPersistMicGain(4.2, null, true)).toBe(true);
  });

  it('does not rewrite a value already established by loaded settings', () => {
    expect(shouldPersistMicGain(4.2, 4.2, false)).toBe(false);
    expect(shouldPersistMicGain(4.2, 4.2, true)).toBe(false);
  });
});
