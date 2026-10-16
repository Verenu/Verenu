export function defaultMicGain(android: boolean): number {
  return android ? 4.5 : 3.5;
}

export function shouldPersistMicGain(
  value: number,
  lastSavedValue: number | null,
  userChanged: boolean,
): boolean {
  return userChanged && value !== lastSavedValue;
}

export const HIGH_MIC_GAIN = 5;

export function micGainAdvice(gain: number, noiseReduction: boolean): string | null {
  if (gain < HIGH_MIC_GAIN) return null;
  if (!noiseReduction) {
    return 'At high gain, background noise is boosted too. Turn on Noise reduction, and raise your system microphone volume instead of this gain where you can.';
  }
  return 'A gain this high usually means the system microphone volume is too low. Raise the input volume in your system sound settings, then lower this slider: boosting here also amplifies room noise and can clip loud speech.';
}
