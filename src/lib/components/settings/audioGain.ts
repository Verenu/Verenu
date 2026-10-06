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
