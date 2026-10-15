import { describe, expect, it } from 'vitest';
import { defaultMicGain, HIGH_MIC_GAIN, micGainAdvice, shouldPersistMicGain } from './audioGain';

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

describe('default mic gain', () => {
  it('starts phones higher than desktops, inside the slider range', () => {
    expect(defaultMicGain(true)).toBeGreaterThan(defaultMicGain(false));
    expect(defaultMicGain(true)).toBeLessThanOrEqual(8);
    expect(defaultMicGain(false)).toBe(3.5);
  });
});

describe('mic gain advice', () => {
  it('stays quiet below the high-gain threshold', () => {
    expect(micGainAdvice(1, false)).toBeNull();
    expect(micGainAdvice(defaultMicGain(true), true)).toBeNull();
    expect(micGainAdvice(HIGH_MIC_GAIN - 0.1, false)).toBeNull();
  });

  it('suggests noise reduction only when it is off', () => {
    expect(micGainAdvice(HIGH_MIC_GAIN, false)).toMatch(/Turn on Noise reduction/);
    expect(micGainAdvice(8, true)).not.toMatch(/Turn on Noise reduction/);
  });

  it('points at the system input volume when noise reduction is already on', () => {
    expect(micGainAdvice(8, true)).toMatch(/system microphone volume is too low/);
  });
});
