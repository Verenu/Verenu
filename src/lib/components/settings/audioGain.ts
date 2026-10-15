/** Starting mic gain with nothing saved. Mirrors `default_mic_gain()` in Rust; phones start higher. */
export function defaultMicGain(android: boolean): number {
  return android ? 4.5 : 3.5;
}

/** Whether a mic-gain value is an explicit value worth writing to settings. */
export function shouldPersistMicGain(
  value: number,
  lastSavedValue: number | null,
  userChanged: boolean,
): boolean {
  return userChanged && value !== lastSavedValue;
}

/** Gain at and above which the slider shows advice. Most microphones need 1-4x. */
export const HIGH_MIC_GAIN = 5;

/**
 * Advice for a boosted microphone, or null when the gain is unremarkable.
 *
 * Software gain multiplies the room noise along with the voice and clips loud
 * speech, so a pinned slider usually means the operating system's input volume
 * is set too low. Noise reduction is only suggested when it is actually off.
 */
export function micGainAdvice(gain: number, noiseReduction: boolean): string | null {
  if (gain < HIGH_MIC_GAIN) return null;
  if (!noiseReduction) {
    return 'At high gain, background noise is boosted too. Turn on Noise reduction, and raise your system microphone volume instead of this gain where you can.';
  }
  return 'A gain this high usually means the system microphone volume is too low. Raise the input volume in your system sound settings, then lower this slider: boosting here also amplifies room noise and can clip loud speech.';
}
