import { describe, expect, it } from 'vitest';
import { customProviderStore, type CustomProvider } from '../../customProviders.svelte';
import { splitModelId, mergeProviderModelMap, providerDisplayLabel } from './models';
import type { ProviderId } from '../../settings';
import type { ModelCatalogCache, ProviderCache } from '../../modelCatalogStore.svelte';
import { supportsLocalLanguage, type Hardware } from './modelPresets';
import {
  curatedRows,
  discoveredRows,
  firstRunnable,
  localModelOffered,
  rowForSelection,
  suggestReplacement,
  unavailableMessages,
  unavailableSelections,
  unverifiedRows,
  type PickerContext,
} from './modelStates';

const T0 = 1_700_000_000_000;

describe('localModelOffered', () => {
  it('keeps Apple Speech and Whisper on hardware where the on-device gate is closed', () => {
    const gated = { supported: false };
    expect(localModelOffered(gated, 'transcription', 'apple-speech')).toBe(true);
    expect(localModelOffered(gated, 'transcription', 'whisper-small')).toBe(true);
    expect(localModelOffered(gated, 'transcription', 'parakeet-tdt-0.6b-v3')).toBe(false);
    expect(localModelOffered(gated, 'transcription', 'fluid-english-booster')).toBe(false);
    expect(localModelOffered(gated, 'cleanup', 'qwen2.5-0.5b-instruct')).toBe(false);
    expect(localModelOffered(gated, 'cleanup', 'whisper-small')).toBe(false);
  });
  it('offers everything when supported', () => {
    expect(localModelOffered({ supported: true }, 'cleanup', 'qwen2.5-0.5b-instruct')).toBe(true);
  });
});

describe('custom providers', () => {
  const provider: CustomProvider = {
    id: 'custom:4c28f3de-9e72-46e8-9f80-2680b9028d1b', name: 'Fixture endpoint', protocol: 'openai',
    base_url: 'http://localhost:8000/v1', requires_key: true,
    supports_cleanup: true, supports_transcription: false, auth_header: null,
    extra_headers: {}, body_overrides: null, transcription_models: [], cleanup_models: ['vendor/model'],
  };
  it('keeps nested model IDs and shows missing keys, disabled tasks, and removed providers', () => {
    customProviderStore.providers = [provider];
    try {
      const id = `${provider.id}/vendor/model`;
      expect(splitModelId(id)).toEqual({ provider: provider.id, model: 'vendor/model' });
      expect(mergeProviderModelMap({ [provider.id]: ['vendor/model'] })[provider.id]).toEqual(['vendor/model']);
      expect(providerDisplayLabel(provider.id)).toBe('Fixture endpoint');
      const context = ctx({ task: 'cleanup' });
      expect(rowForSelection(id, context)?.state).toBe('needs-setup');
      expect(curatedRows(context).find(r => r.key === id)?.remedy).toBe('add-key');
      context.apiKeyStatus[provider.id] = true;
      expect(rowForSelection(id, context)?.state).toBe('ready');
      expect(rowForSelection(id, { ...context, task: 'transcription' })?.state).toBe('unavailable');
      customProviderStore.providers = [];
      expect(rowForSelection(id, context)?.note).toBe('Provider removed');
    } finally { customProviderStore.providers = []; }
  });
  it('makes keyless models selectable without adding a key', () => {
    customProviderStore.providers = [{ ...provider, requires_key: false }];
    try {
      expect(curatedRows(ctx({ task: 'cleanup' })).find(r => r.provider === provider.id)?.state).toBe('ready');
      const id = `${provider.id}/vendor/model`;
      const context = ctx({ task: 'cleanup' });
      expect(rowForSelection(id, context)?.state).toBe('ready');
      expect(rowForSelection(id, context)?.remedy).toBe('none');
      expect(firstRunnable([id], context)).toBe(id);
      expect(rowForSelection(id, { ...context, task: 'transcription' })?.state).toBe('unavailable');
      customProviderStore.providers = [];
      expect(rowForSelection(id, context)?.note).toBe('Provider removed');
    } finally { customProviderStore.providers = []; }
  });
});
const CAPABLE: Hardware = { totalRamMb: 32768, freeRamMb: 24576, gpus: [], unknown: false };
const TINY: Hardware = { totalRamMb: 4096, freeRamMb: 2048, gpus: [], unknown: false };

const ALL_KEYS: Record<ProviderId, boolean> = {
  groq: true,
  openai: true,
  google: true,
  assemblyai: true,
  openrouter: true,
  xai: true,
  local: true,
};

function providerCache(overrides: Partial<ProviderCache> = {}): ProviderCache {
  return {
    ids: [],
    everSeen: [],
    lastSuccessAt: T0,
    lastAttemptAt: T0,
    lastError: null,
    missing: {},
    ...overrides,
  };
}

/** Two confirmed misses — the threshold `unavailableSelections` looks for. */
function missed(id: string) {
  return { [id]: { count: 2, lastCountedAt: T0 } };
}

function ctx(overrides: Partial<PickerContext> = {}): PickerContext {
  return {
    task: 'transcription',
    apiKeyStatus: { ...ALL_KEYS },
    cache: {},
    localModels: [],
    hardware: CAPABLE,
    ...overrides,
  };
}

describe('curatedRows', () => {
  it('marks a keyless provider as needing setup, not as gone', () => {
    const rows = curatedRows(ctx({ apiKeyStatus: { ...ALL_KEYS, openai: false } }));
    const row = rows.find((r) => r.key === 'openai/gpt-4o-transcribe')!;
    expect(row.state).toBe('needs-setup');
    expect(row.remedy).toBe('add-key');
  });

  it('offers a keyed provider before any list has landed, and says so', () => {
    const row = curatedRows(ctx()).find((r) => r.key === 'groq/whisper-large-v3')!;
    expect(row.state).toBe('ready');
    expect(row.note).toMatch(/Not verified against Groq yet/);
  });

  it('trusts a live list over the mere presence of a key', () => {
    const cache: ModelCatalogCache = { groq: providerCache({ ids: ['whisper-large-v3'], missing: missed('groq/whisper-large-v3-turbo') }) };
    const rows = curatedRows(ctx({ cache }));
    expect(rows.find((r) => r.key === 'groq/whisper-large-v3')!.state).toBe('ready');
    // A model the provider no longer lists simply drops out — the picker shows
    // what you can pick, and a retired model is not one of those.
    expect(rows.find((r) => r.key === 'groq/whisper-large-v3-turbo')).toBeUndefined();
  });

  it('keeps a retired model listed while it is still selected', () => {
    const cache: ModelCatalogCache = { groq: providerCache({ ids: ['whisper-large-v3'], missing: missed('groq/whisper-large-v3-turbo') }) };
    const rows = curatedRows(ctx({ cache }), ['groq/whisper-large-v3-turbo']);
    // Otherwise a dead selection has nowhere to show its state or be replaced.
    expect(rows.find((r) => r.key === 'groq/whisper-large-v3-turbo')?.state).toBe('unavailable');
  });

  it('falls back to ready-by-key when the last fetch failed', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({ ids: ['whisper-large-v3'], lastError: 'offline' }),
    };
    const rows = curatedRows(ctx({ cache }));
    expect(rows.find((r) => r.key === 'groq/whisper-large-v3-turbo')!.state).toBe('ready');
  });

  it('separates not-downloaded from will-not-fit for local models', () => {
    const notDownloaded = curatedRows(
      ctx({ localModels: [{ id: 'parakeet-v3', is_downloaded: false, size_mb: 456 }] }),
    ).find((r) => r.key === 'local/parakeet-v3')!;
    expect(notDownloaded.remedy).toBe('download');

    const tooBig = curatedRows(
      ctx({
        hardware: TINY,
        localModels: [{ id: 'parakeet-v3', is_downloaded: true, size_mb: 4000 }],
      }),
    ).find((r) => r.key === 'local/parakeet-v3')!;
    expect(tooBig.state).toBe('needs-setup');
    expect(tooBig.note).toMatch(/memory/);
    expect(tooBig.remedy).toBe('none');
  });
});

describe('local speech models', () => {
  const listed = [
    { id: 'whisper-small', is_downloaded: false, size_mb: 465 },
    { id: 'apple-speech', install_kind: 'system_managed' as const, is_downloaded: true, size_mb: 0 },
  ];

  it('offers listed Whisper for download and Apple Speech without one', () => {
    const rows = curatedRows(ctx({ localModels: listed }));
    expect(rows.find((r) => r.key === 'local/whisper-small')).toMatchObject({ state: 'needs-setup', remedy: 'download' });
    expect(rows.find((r) => r.key === 'local/apple-speech')).toMatchObject({ state: 'ready', remedy: 'none' });
  });

  it('hides models the backend does not list, but keeps a pinned pick replaceable', () => {
    const context = ctx({ localModels: [{ id: 'parakeet-v3', is_downloaded: true, size_mb: 456 }] });
    expect(curatedRows(context).some((r) => r.id.startsWith('fluid-') || r.id === 'apple-speech')).toBe(false);
    const pinned = curatedRows(context, ['local/apple-speech']).find((r) => r.key === 'local/apple-speech')!;
    expect(pinned.state).toBe('unavailable');
    expect(pinned.note).toMatch(/choose another model/);
  });

  it('treats backend wildcard languages as the frontend subset, not as unsupported', () => {
    const localModels = [
      { id: 'whisper-small', supported_languages: ['Multilingual'] },
      { id: 'fluid-parakeet-ultra', supported_languages: ['Multilingual'] },
    ] as never;
    expect(supportsLocalLanguage('whisper-small', { language: 'ja', localModels })).toBe(true);
    expect(supportsLocalLanguage('fluid-parakeet-ultra', { language: 'fr', localModels })).toBe(true);
    expect(supportsLocalLanguage('fluid-parakeet-ultra', { language: 'ja', localModels })).toBe(false);
  });

  it('never lists the auxiliary booster as a speech model', () => {
    const rows = curatedRows(ctx({ localModels: [{ id: 'fluid-english-booster', is_downloaded: true, size_mb: 98 }] }));
    expect(rows.some((r) => r.id === 'fluid-english-booster')).toBe(false);
    for (const language of [undefined, 'auto', 'en']) {
      expect(supportsLocalLanguage('fluid-english-booster', { language })).toBe(false);
      const localModels = [{ id: 'another-booster', engine_type: 'ctc_booster', supported_languages: ['English'] }] as never;
      expect(supportsLocalLanguage('another-booster', { language, localModels })).toBe(false);
    }
  });

  it('explains an unsupported default and names the fallback', () => {
    const context = ctx({ localModels: [{ id: 'parakeet-v3', is_downloaded: true, size_mb: 456 }] });
    const [message] = unavailableMessages('transcription', 'local/apple-speech', ['local/parakeet-v3'], context);
    expect(message).toMatch(/isn't available on this device/);
    expect(message).toContain('Local Parakeet V3');
  });
});

describe('unverifiedRows', () => {
  it('automatically offers compatible discoveries for all cloud providers', () => {
    const cache: ModelCatalogCache = {
      google: providerCache({ ids: ['gemini-new'], metadata: { 'gemini-new': { label: 'New Gemini', tasks: ['transcription', 'cleanup'] } } }),
      openrouter: providerCache({ ids: ['org/new:free'], metadata: { 'org/new:free': { label: 'New Router model', tasks: ['cleanup'] } } }),
      xai: providerCache({ ids: ['grok-new'], metadata: { 'grok-new': { label: 'New Grok', tasks: ['cleanup'] } } }),
    };
    expect(discoveredRows(ctx({ cache })).map((row) => row.key)).toEqual(['google/gemini-new']);
    expect(discoveredRows(ctx({ task: 'cleanup', cache })).map((row) => row.key)).toEqual(['google/gemini-new', 'openrouter/org/new:free', 'xai/grok-new']);
    expect(unverifiedRows(ctx({ task: 'cleanup', cache }))).toEqual([]);
  });

  it('keeps offline capabilities and requests a key for public discoveries', () => {
    const cache: ModelCatalogCache = { openrouter: providerCache({ ids: ['org/new'], lastError: 'offline', metadata: { 'org/new': { label: 'New', tasks: ['cleanup'] } } }) };
    const context = ctx({ task: 'cleanup', cache });
    expect(discoveredRows(context)[0].state).toBe('ready');
    expect(discoveredRows(context)[0].note).toMatch(/Cached/);
    context.apiKeyStatus.openrouter = false;
    expect(discoveredRows(context)[0].remedy).toBe('add-key');
    expect(rowForSelection('openrouter/org/new', context)?.state).toBe('needs-setup');
  });

  it('does not offer known incompatible metadata in the unverified tail', () => {
    const cache: ModelCatalogCache = { openrouter: providerCache({ ids: ['org/special'], metadata: { 'org/special': { label: 'Special', tasks: [] } } }) };
    expect(unverifiedRows(ctx({ task: 'cleanup', cache }))).toEqual([]);
    expect(discoveredRows(ctx({ task: 'cleanup', cache }))).toEqual([]);
  });

  it('keeps models after one miss while waiting for confirmation', () => {
    const cache: ModelCatalogCache = { groq: providerCache({ ids: ['whisper-large-v3'], missing: { 'groq/whisper-large-v3-turbo': { count: 1, lastCountedAt: T0 } } }) };
    expect(curatedRows(ctx({ cache })).find((row) => row.id === 'whisper-large-v3-turbo')?.state).toBe('ready');
  });
  it('only surfaces ids the catalog does not know', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({ ids: ['whisper-large-v3', 'distil-whisper-large-v3-en'] }),
    };
    const rows = unverifiedRows(ctx({ cache }));
    expect(rows.map((r) => r.id)).toEqual(['distil-whisper-large-v3-en']);
  });

  it('splits transcription from cleanup by name, best-effort', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({ ids: ['some-transcribe-model', 'some-chat-model'] }),
    };
    expect(unverifiedRows(ctx({ cache })).map((r) => r.id)).toEqual(['some-transcribe-model']);
    expect(unverifiedRows(ctx({ task: 'cleanup', cache })).map((r) => r.id)).toEqual([
      'some-chat-model',
    ]);
  });

  it('filters non-task modalities with word-boundary model names', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({
        ids: ['model-stt', 'tts-1', 'text-embedding-3-small', 'llama-chat'],
      }),
    };
    expect(unverifiedRows(ctx({ cache })).map((r) => r.id)).toEqual(['model-stt']);
    expect(unverifiedRows(ctx({ task: 'cleanup', cache })).map((r) => r.id)).toEqual(['llama-chat']);
  });

  it('retains cached discoveries while a provider is offline', () => {
    const cache: ModelCatalogCache = { groq: providerCache({ ids: ['new-transcribe'], lastError: 'boom' }) };
    expect(unverifiedRows(ctx({ cache })).map((row) => row.id)).toEqual(['new-transcribe']);
  });
});

describe('rowForSelection', () => {
  const cache: ModelCatalogCache = {
    groq: providerCache({
      ids: ['whisper-large-v3'],
      everSeen: ['whisper-large-v3', 'retired-model'],
    }),
  };

  it('distinguishes a retired model from one that never existed', () => {
    expect(rowForSelection('groq/retired-model', ctx({ cache }))!.state).toBe('unavailable');
    expect(rowForSelection('groq/whisper-larg-v3', ctx({ cache }))!.state).toBe('not-found');
  });

  it('still returns a row for a live custom id', () => {
    const row = rowForSelection('groq/whisper-large-v3', ctx({ cache }))!;
    expect(row.state).toBe('ready');
  });

  it('returns null for an unparseable id', () => {
    expect(rowForSelection('', ctx())).toBeNull();
    expect(rowForSelection('no-slash', ctx())).toBeNull();
  });
});

describe('unavailableSelections', () => {
  const selected = ['groq/whisper-large-v3', 'openai/gpt-4o-transcribe'];

  it('stays silent when the last fetch failed', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({ ids: [], lastError: 'offline', missing: missed('groq/whisper-large-v3') }),
    };
    expect(unavailableSelections('transcription', selected, ctx({ cache }))).toEqual([]);
  });

  it('stays silent when no fetch has ever succeeded', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({ lastSuccessAt: 0, missing: missed('groq/whisper-large-v3') }),
    };
    expect(unavailableSelections('transcription', selected, ctx({ cache }))).toEqual([]);
  });

  it('stays silent on a single miss', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({
        ids: [],
        everSeen: ['whisper-large-v3'],
        missing: { 'groq/whisper-large-v3': { count: 1, lastCountedAt: T0 } },
      }),
    };
    expect(unavailableSelections('transcription', selected, ctx({ cache }))).toEqual([]);
  });

  it('flags a confirmed miss and suggests a live sibling', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({
        ids: ['whisper-large-v3-turbo'],
        everSeen: ['whisper-large-v3', 'whisper-large-v3-turbo'],
        missing: missed('groq/whisper-large-v3'),
      }),
    };
    const [found] = unavailableSelections('transcription', selected, ctx({ cache }));
    expect(found.reason).toBe('deprecated');
    expect(found.position).toBe('default');
    expect(found.suggestion).toBe('groq/whisper-large-v3-turbo');
  });

  it('never flags local or AssemblyAI models', () => {
    const cache: ModelCatalogCache = {
      local: providerCache({ ids: [], missing: missed('local/parakeet-v3') }),
      assemblyai: providerCache({ ids: [], missing: missed('assemblyai/universal-2') }),
    };
    const picks = ['local/parakeet-v3', 'assemblyai/universal-2'];
    expect(unavailableSelections('transcription', picks, ctx({ cache }))).toEqual([]);
  });

  it('reports a fallback by its position in the chain', () => {
    const cache: ModelCatalogCache = {
      openai: providerCache({
        ids: [],
        everSeen: ['gpt-4o-transcribe'],
        missing: missed('openai/gpt-4o-transcribe'),
      }),
    };
    const [found] = unavailableSelections('transcription', selected, ctx({ cache }));
    expect(found.position).toBe(0);
  });
});

describe('suggestReplacement', () => {
  it('prefers a sibling sharing the most tags', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({ ids: ['whisper-large-v3-turbo', 'whisper-large-v3'] }),
    };
    expect(suggestReplacement('transcription', 'groq', 'whisper-large-v3', ctx({ cache }))).toBe(
      'groq/whisper-large-v3-turbo',
    );
  });

  it('returns null for a model the catalog never described', () => {
    expect(suggestReplacement('transcription', 'groq', 'mystery-model', ctx())).toBeNull();
  });
});

describe('unavailableMessages', () => {
  const goneGroq = (): ModelCatalogCache => ({
    groq: providerCache({
      ids: [],
      everSeen: ['whisper-large-v3'],
      missing: missed('groq/whisper-large-v3'),
    }),
  });

  it('names the fallback that will pick up the work', () => {
    const cache = { ...goneGroq(), openai: providerCache({ ids: ['gpt-4o-transcribe'] }) };
    const [message] = unavailableMessages(
      'transcription',
      'groq/whisper-large-v3',
      ['openai/gpt-4o-transcribe'],
      ctx({ cache }),
    );
    expect(message).toContain('no longer offered by Groq');
    expect(message).toContain('OpenAI GPT-4o Transcribe');
  });

  it('warns outright when nothing in the chain can run', () => {
    const [message] = unavailableMessages(
      'transcription',
      'groq/whisper-large-v3',
      [],
      ctx({ cache: goneGroq(), apiKeyStatus: { ...ALL_KEYS, openai: false, google: false } }),
    );
    expect(message).toMatch(/no fallback is usable/);
    expect(message).toMatch(/transcription will fail/);
  });

  it('says a dead fallback will be skipped, not that the task changes model', () => {
    const cache = { ...goneGroq(), openai: providerCache({ ids: ['gpt-4o-transcribe'] }) };
    const [message] = unavailableMessages(
      'transcription',
      'openai/gpt-4o-transcribe',
      ['groq/whisper-large-v3'],
      ctx({ cache }),
    );
    expect(message).toBe(
      'Fallback #1 Whisper Large v3 is no longer offered by Groq and will be skipped.',
    );
  });

  it('does not claim a never-seen id was retired', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({ ids: [], everSeen: [], missing: missed('groq/whisper-larg-v3') }),
    };
    const [message] = unavailableMessages('transcription', 'groq/whisper-larg-v3', [], ctx({ cache }));
    expect(message).toMatch(/can't find/);
    expect(message).not.toMatch(/no longer offered/);
  });

  it('says nothing at all when every selection is live', () => {
    const cache: ModelCatalogCache = { groq: providerCache({ ids: ['whisper-large-v3'] }) };
    expect(unavailableMessages('transcription', 'groq/whisper-large-v3', [], ctx({ cache }))).toEqual(
      [],
    );
  });
});

describe('firstRunnable', () => {
  it('skips over models that cannot run right now', () => {
    const cache: ModelCatalogCache = {
      groq: providerCache({ ids: [], missing: missed('groq/whisper-large-v3') }),
      openai: providerCache({ ids: ['gpt-4o-transcribe'] }),
    };
    const chain = ['groq/whisper-large-v3', 'openai/gpt-4o-transcribe'];
    expect(firstRunnable(chain, ctx({ cache }))).toBe('openai/gpt-4o-transcribe');
  });

  it('returns null when nothing in the chain is usable', () => {
    expect(firstRunnable([], ctx())).toBeNull();
  });
});
