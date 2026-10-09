import { describe, expect, it, vi } from 'vitest';
import { buildPresets, type Hardware } from '../components/settings/modelPresets';
import { reconcilePresetSelection } from './presetSelection';

vi.mock('../platform', () => ({ isAndroid: false }));
vi.mock('../tauri', () => ({ invoke: vi.fn() }));

const noKeys = { groq: false, openai: false, google: false, assemblyai: false, openrouter: false, xai: false, local: false };
const desktop: Hardware = { totalRamMb: 16384, freeRamMb: 12288, gpus: [], unknown: false, isAndroid: false };

describe('setup preset selection reconciliation', () => {
  it('keeps the selected cloud priority and adopts its target after provider keys change', () => {
    const selected = buildPresets({ ...noKeys, groq: true }, desktop, false)
      .find((preset) => preset.id === 'cloud-balanced')!;
    const available = buildPresets({ ...noKeys, openai: true }, desktop, false);
    const next = reconcilePresetSelection(selected, available)!;

    expect(next).toBe(available.find((preset) => preset.id === 'cloud-balanced'));
    expect(next.id).toBe(selected.id);
    expect(next.target).not.toEqual(selected.target);
    expect(next.target?.transcriptionDefaultModel).toMatch(/^openai\//);
  });

  it('keeps an equivalent selected preset stable and preserves local-only targets when cloud keys appear', () => {
    const selected = buildPresets(noKeys, desktop, true)
      .find((preset) => preset.id === 'local-balanced')!;
    const available = buildPresets({ ...noKeys, groq: true }, desktop, true);
    const next = reconcilePresetSelection(selected, available)!;

    expect(next).toBe(selected);
    expect(next.offline).toBe(true);
    expect([
      next.target!.transcriptionDefaultModel,
      next.target!.cleanupDefaultModel!,
      ...next.target!.transcriptionFallbacks,
      ...next.target!.cleanupFallbacks,
    ].every((model) => model.startsWith('local/'))).toBe(true);
  });

  it('clears a selection when that preset is no longer available', () => {
    const selected = buildPresets({ ...noKeys, groq: true }, desktop, false)
      .find((preset) => preset.id === 'cloud-balanced')!;

    expect(reconcilePresetSelection(selected, [])).toBeNull();
  });
});
