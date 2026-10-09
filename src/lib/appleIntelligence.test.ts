import { describe, it, expect, vi, beforeEach } from 'vitest';
import { appleIntelligenceVisible, curatedRows, rowForSelection, unverifiedRows, type PickerContext } from './components/settings/modelStates';
import { emptyProviderModelMap, mergeProviderModelMap, splitModelId } from './components/settings/models';
import { dictationReadiness, type ReadinessInput } from './dictationReadiness';
import { appleIntelligence, refreshAppleIntelligence } from './appleIntelligence.svelte';
import { invoke } from './tauri';

vi.mock('./tauri', () => ({ invoke: vi.fn() }));

const status = { state: 'available', available: true, message: 'On-device cleanup' };
const picker: PickerContext = {
  task: 'cleanup', apiKeyStatus: { groq: false, openai: false, google: false, assemblyai: false, openrouter: false, xai: false, local: false, 'apple-intelligence': false },
  cache: {}, localModels: [], hardware: { totalRamMb: 16384, freeRamMb: 12000, gpus: [], unknown: false }, appleIntelligence: status,
};
const readiness: ReadinessInput = {
  transcriptionModel: 'local/parakeet-v3', cleanupModel: 'apple-intelligence/system', cleanupEnabled: true,
  keys: {}, speechModels: [{ id: 'parakeet-v3', is_downloaded: true }], cleanupModels: [], cleanupEngineInstalled: false,
  appleIntelligence: status, isOnline: false,
};

describe('Apple Intelligence cleanup', () => {
  it('preserves the provider/model identity and model map independently of existing selections', () => {
    expect(splitModelId('apple-intelligence/system')).toEqual({ provider: 'apple-intelligence', model: 'system' });
    expect(mergeProviderModelMap({ groq: ['existing'], 'apple-intelligence': ['system'] })).toEqual({ ...emptyProviderModelMap(), groq: ['existing'], 'apple-intelligence': ['system'] });
  });
  it('is cleanup-only, keyless, offline and independent of downloaded GGUF models', () => {
    expect(rowForSelection('apple-intelligence/system', picker)?.state).toBe('ready');
    expect(rowForSelection('apple-intelligence/system', picker)?.remedy).toBe('none');
    expect(curatedRows({ ...picker, task: 'transcription' }).some(row => row.provider === 'apple-intelligence')).toBe(false);
    expect(rowForSelection('apple-intelligence/system', { ...picker, task: 'transcription' })?.state).toBe('unavailable');
    expect(dictationReadiness(readiness)).toEqual([]);
  });
  it.each(['intelligence-disabled', 'model-not-ready'])('shows %s on a supported Mac as actionable setup without an API key prompt', state => {
    const setup = { state, available: false, message: `Recovery for ${state}` };
    const ctx = { ...picker, appleIntelligence: setup };
    const row = curatedRows(ctx, ['apple-intelligence/system']).find(item => item.provider === 'apple-intelligence');
    expect(row?.key).toBe('apple-intelligence/system');
    expect(row?.state).toBe('needs-setup');
    expect(row?.note).toBe(setup.message);
    expect(row?.remedy).toBe('none');
    const issues = dictationReadiness({ ...readiness, appleIntelligence: setup });
    expect(issues).toHaveLength(1);
    expect(issues[0].section).toBe('models');
    expect(issues[0].message).toBe(setup.message);
  });
  it.each(['unsupported-os', 'device-not-eligible', 'unsupported-platform', 'unavailable', 'checking', 'unknown'])('hides Apple Intelligence from picker rows and the rail for %s, even when pinned', state => {
    const hidden = { state, available: false, message: `Recovery for ${state}` };
    const ctx = { ...picker, appleIntelligence: hidden };
    expect(appleIntelligenceVisible(ctx)).toBe(false);
    expect(curatedRows(ctx, ['apple-intelligence/system']).some(row => row.provider === 'apple-intelligence')).toBe(false);
    expect(curatedRows(ctx).some(row => row.provider === 'apple-intelligence')).toBe(false);
    expect(unverifiedRows(ctx).some(row => row.provider === 'apple-intelligence')).toBe(false);
    // Readiness still reports the stored choice, and never requests an API key.
    const issues = dictationReadiness({ ...readiness, appleIntelligence: hidden });
    expect(issues).toHaveLength(1);
    expect(issues[0].section).toBe('models');
  });
  it('rejects unknown Apple models and speech selection even when available', () => {
    expect(rowForSelection('apple-intelligence/unknown', picker)?.state).toBe('unavailable');
    expect(dictationReadiness({ ...readiness, transcriptionModel: 'apple-intelligence/system' })[0].task).toBe('transcription');
  });
});

describe('refreshAppleIntelligence', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });
  it('stores the backend availability and returns it for readiness', async () => {
    const backend = { state: 'model-not-ready', available: false, message: 'Apple Intelligence is still preparing its model.' };
    vi.mocked(invoke).mockResolvedValueOnce(backend);
    await expect(refreshAppleIntelligence()).resolves.toEqual(backend);
    expect(invoke).toHaveBeenCalledWith('get_apple_intelligence_availability');
    expect(appleIntelligence.status).toEqual(backend);
  });
  it('reports unavailable without a key prompt when the check fails', async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error('boom'));
    const status = await refreshAppleIntelligence();
    expect(status.state).toBe('unavailable');
    expect(status.available).toBe(false);
    expect(appleIntelligence.status).toEqual(status);
  });
});
