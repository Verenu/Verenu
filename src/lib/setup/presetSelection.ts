import type { Preset } from '../components/settings/modelPresets';

export function reconcilePresetSelection(current: Preset | null, available: readonly Preset[]): Preset | null {
  if (!current) return null;
  const latest = available.find((candidate) => candidate.kind === 'preset' && candidate.id === current.id);
  if (!latest) return null;
  return JSON.stringify(current) === JSON.stringify(latest) ? current : latest;
}
