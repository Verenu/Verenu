/** Whether a mic-gain value is an explicit value worth writing to settings. */
export function shouldPersistMicGain(
  value: number,
  lastSavedValue: number | null,
  userChanged: boolean,
): boolean {
  return userChanged && value !== lastSavedValue;
}
