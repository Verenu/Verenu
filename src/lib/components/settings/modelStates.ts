import type { ProviderId } from '../../settings';
import { appleIntelligence, appleIntelligenceSupported, type AppleIntelligenceAvailability } from '../../appleIntelligence.svelte';
import { customProviderStore, customProvider, isCustomProviderId } from '../../customProviders.svelte';
import {
  isTrustworthy,
  hasSnapshot,
  CLOUD_PROVIDERS,
  MISS_INTERVAL_MS,
  type ModelCatalogCache,
} from '../../modelCatalogStore.svelte';
import { fitsHardware, type Hardware } from './modelPresets';
import {
  CATALOG,
  catalogEntry,
  catalogFor,
  modelDisplayLabel,
  modelId,
  providerDisplayLabel,
  qualifiedModelLabel,
  splitModelId,
  type CatalogEntry,
  type ModelTag,
  type TaskType,
} from './models';

export type ModelState = 'ready' | 'needs-setup' | 'unavailable' | 'not-found' | 'unverified';

export type Remedy = 'add-key' | 'download' | 'none';

export type ModelRow = {
  provider: ProviderId;
  id: string;
  key: string;
  label: string;
  tags: ModelTag[];
  state: ModelState;
  note: string;
  remedy: Remedy;
  sizeMb?: number;
};

export type LocalModelInfo = {
  install_kind?: 'system_managed' | 'download';
  id: string;
  name?: string;
  description?: string;
  is_downloaded?: boolean;
  is_downloading?: boolean;
  is_recommended?: boolean;
  size_mb?: number;
  quantization?: string;
  prompt_family?: string;
};

export type LocalControls = {
  models: LocalModelInfo[];
  supported: boolean;
  downloadProgress: Record<string, { progress?: number } | undefined>;
  downloadStage: Record<string, string | undefined>;
  onDownload: (id: string) => void;
  onCancel: (id: string) => void;
  onDelete: (id: string) => void;
  runtime?: {
    info?: { installed: boolean; backend?: string; approx_download_mb?: number; is_downloading?: boolean };
    progress?: { stage?: string; progress?: number };
    onDownload: () => void;
    onCancel: () => void;
    onDelete: () => void;
  };
};

export type PickerContext = {
  task: TaskType;
  apiKeyStatus: Record<ProviderId, boolean>;
  cache: ModelCatalogCache;
  localModels: LocalModelInfo[];
  hardware: Hardware;
  appleIntelligence?: AppleIntelligenceAvailability;
};

export function appleIntelligenceVisible(ctx: Pick<PickerContext, 'appleIntelligence'>): boolean {
  return appleIntelligenceSupported(ctx.appleIntelligence ?? appleIntelligence.status);
}

const LISTED_PROVIDERS = CLOUD_PROVIDERS;
const GOOGLE_DEDICATED_TRANSCRIBER = 'gemini-3.5-transcribe';

function row(entry: CatalogEntry, state: ModelState, note = '', remedy: Remedy = 'none'): ModelRow {
  return {
    provider: entry.provider,
    id: entry.id,
    key: modelId(entry.provider, entry.id),
    label: entry.label,
    tags: entry.tags,
    state,
    note,
    remedy,
  };
}

const PLATFORM_GATED_LOCAL = (id: string) =>
  id === 'apple-speech' || id.startsWith('fluid-') || id.startsWith('whisper-');
const UNSUPPORTED_LOCAL_NOTE = 'Not available on this device · choose another model';

const isPortableLocalSpeech = (id: string) => id === 'apple-speech' || id.startsWith('whisper-');

export function localModelOffered(local: Pick<LocalControls, 'supported'>, task: TaskType, id: string): boolean {
  return local.supported || (task === 'transcription' && isPortableLocalSpeech(id));
}

function localRow(entry: CatalogEntry, ctx: PickerContext): ModelRow {
  const info = ctx.localModels.find((model) => model.id === entry.id);
  const sizeMb = info?.size_mb;
  const base = { ...row(entry, 'ready'), sizeMb };
  if (!info && entry.tasks.includes('transcription') && PLATFORM_GATED_LOCAL(entry.id)) {
    return { ...base, state: 'unavailable', note: UNSUPPORTED_LOCAL_NOTE };
  }

  if (info?.install_kind === 'system_managed') {
    return { ...base, note: 'Managed by macOS · permission and language assets required' };
  }
  if (!info || info.is_downloaded !== true) {
    return { ...base, state: 'needs-setup', note: 'Not downloaded', remedy: 'download' };
  }
  if (sizeMb !== undefined && !fitsHardware(ctx.hardware, [sizeMb])) {
    return { ...base, state: 'needs-setup', note: 'Needs more memory than this machine has' };
  }
  return { ...base, note: 'Installed' };
}

function unsupportedLocalPick(id: string, ctx: PickerContext): boolean {
  const parsed = splitModelId(id);
  if (parsed?.provider !== 'local' || ctx.localModels.length === 0) return false;
  const entry = catalogEntry('local', parsed.model);
  return !!entry && localRow(entry, ctx).note === UNSUPPORTED_LOCAL_NOTE;
}

function cloudRow(entry: CatalogEntry, ctx: PickerContext): ModelRow {
  if (entry.provider === 'apple-intelligence') {
    const status = ctx.appleIntelligence ?? appleIntelligence.status;
    if (ctx.task !== 'cleanup' || entry.id !== 'system') return row(entry, 'unavailable', 'Cleanup only');
    return row(entry, status.available ? 'ready' : 'needs-setup', status.message);
  }
  if (!ctx.apiKeyStatus[entry.provider] && customProvider(entry.provider)?.requires_key !== false) {
    return row(entry, 'needs-setup', 'No API key', 'add-key');
  }

  if (entry.provider === 'google' && entry.id === GOOGLE_DEDICATED_TRANSCRIBER) {
    return row(entry, 'ready');
  }

  const cache = ctx.cache[entry.provider];
  if (!isTrustworthy(cache)) {
    return row(entry, 'ready', `Not verified against ${providerDisplayLabel(entry.provider)} yet`);
  }
  if (cache!.ids.includes(entry.id)) return row(entry, 'ready');
  if ((cache!.missing[modelId(entry.provider, entry.id)]?.count ?? 0) < 2) return row(entry, 'ready', 'Not in the latest list; keeping this model until confirmed');

  return row(entry, 'unavailable', `No longer offered by ${providerDisplayLabel(entry.provider)}`);
}

export function curatedRows(ctx: PickerContext, keep: string[] = []): ModelRow[] {
  const pinned = new Set(keep);
  const customEntries: CatalogEntry[] = customProviderStore.providers
    .filter(p => ctx.task === 'transcription' ? p.supports_transcription : p.supports_cleanup)
    .flatMap(p => (ctx.task === 'transcription' ? p.transcription_models : p.cleanup_models)
      .map(id => ({ provider: p.id, id, label: id, tasks: [ctx.task], tags: [] })));
  return [...catalogFor(ctx.task), ...customEntries]
    .filter((entry) => entry.provider !== 'apple-intelligence' || appleIntelligenceVisible(ctx))
    .map((entry) => (entry.provider === 'local' ? localRow(entry, ctx) : cloudRow(entry, ctx)))
    .filter((row) => row.state !== 'unavailable' || pinned.has(row.key));
}

export function discoveredRows(ctx: PickerContext): ModelRow[] {
  const rows: ModelRow[] = [];
  for (const provider of LISTED_PROVIDERS) {
    const cache = ctx.cache[provider];
    if (!hasSnapshot(cache)) continue;
    for (const id of cache!.ids) {
      if (catalogEntry(provider, id)) continue;
      const info = cache!.metadata?.[id];
      if (!info?.tasks.includes(ctx.task)) continue;
      const keyed = ctx.apiKeyStatus[provider];
      rows.push({ provider, id, key: modelId(provider, id), label: info.label || id,
        tags: [], state: keyed ? 'ready' : 'needs-setup', remedy: keyed ? 'none' : 'add-key',
        note: keyed ? (cache!.lastError ? 'Cached model · refresh unavailable' : 'Listed model · capability metadata') : 'No API key' });
    }
  }
  return rows;
}

/**
 * Ids a provider returned that aren't in the catalog.
 *
 * Groq's and OpenAI's lists have no modality fields. IDs without capability
 * metadata use a name heuristic and remain under Show more models.
 */
export function unverifiedRows(ctx: PickerContext): ModelRow[] {
  const transcriptionish = /whisper|transcribe|speech|[-/]stt\b/i;
  const otherModality =
    /\b(tts|text-to-speech|embed|embedding|rerank|moderation|guard|safeguard|image|vision|video|veo|imagen|lyria|dall-e|sora|robotics|orpheus|playai)\b/i;
  const rows: ModelRow[] = [];

  for (const provider of LISTED_PROVIDERS) {
    const cache = ctx.cache[provider];
    if (!hasSnapshot(cache)) continue;
    for (const id of cache!.ids) {
      if (catalogEntry(provider, id)) continue;
      if (cache!.metadata?.[id]) continue;
      if (otherModality.test(id)) continue;
      const looksTranscription = transcriptionish.test(id);
      if (ctx.task === 'transcription' ? !looksTranscription : looksTranscription) continue;
      rows.push({
        provider,
        id,
        key: modelId(provider, id),
        label: id,
        tags: [],
        state: ctx.apiKeyStatus[provider] ? 'unverified' : 'needs-setup',
        note: ctx.apiKeyStatus[provider] ? 'Not verified for this task' : 'No API key',
        remedy: ctx.apiKeyStatus[provider] ? 'none' : 'add-key',
      });
    }
  }

  return rows;
}

export function rowForSelection(selectedId: string, ctx: PickerContext): ModelRow | null {
  const parsed = splitModelId(selectedId);
  if (!parsed) return null;

  const entry = catalogEntry(parsed.provider, parsed.model);
  if (entry) {
    return entry.provider === 'local' ? localRow(entry, ctx) : cloudRow(entry, ctx);
  }

  const cache = ctx.cache[parsed.provider];
  const discovered = discoveredRows(ctx).find((row) => row.key === selectedId);
  if (discovered) return discovered;
  const base: ModelRow = {
    provider: parsed.provider,
    id: parsed.model,
    key: selectedId,
    label: modelDisplayLabel(parsed.provider, parsed.model),
    tags: [],
    state: 'unverified',
    note: 'Custom model',
    remedy: 'none',
  };
  if (parsed.provider === 'apple-intelligence') {
    return { ...base, state: 'unavailable', note: 'Unsupported Apple Intelligence model' };
  }
  if (isCustomProviderId(parsed.provider)) {
    const p = customProvider(parsed.provider);
    if (!p || !(ctx.task === 'transcription' ? p.supports_transcription : p.supports_cleanup))
      return { ...base, state: 'unavailable', note: p ? 'Task disabled for this provider' : 'Provider removed' };
    if (p.requires_key && !ctx.apiKeyStatus[p.id])
      return { ...base, state: 'needs-setup', note: 'No API key', remedy: 'add-key' };
    return { ...base, state: 'ready', note: 'User-defined endpoint' };
  }
  if (parsed.provider !== 'local' && !ctx.apiKeyStatus[parsed.provider]) {
    return { ...base, state: 'needs-setup', note: 'No API key', remedy: 'add-key' };
  }
  if (parsed.provider === 'local' || !isTrustworthy(cache)) return base;
  if (cache!.ids.includes(parsed.model)) return base;

  return cache!.everSeen.includes(parsed.model)
    ? { ...base, state: 'unavailable', note: `No longer offered by ${providerDisplayLabel(parsed.provider)}` }
    : { ...base, state: 'not-found', note: `Not in ${providerDisplayLabel(parsed.provider)}'s model list` };
}

export type UnavailableReason = 'deprecated' | 'not-found';

export type UnavailableSelection = {
  id: string;
  provider: ProviderId;
  model: string;
  reason: UnavailableReason;
  position: 'default' | number;
  suggestion: string | null;
};

function confirmedMissing(cache: ModelCatalogCache[ProviderId], id: string): boolean {
  const counter = cache?.missing[id];
  return !!counter && counter.count >= 2;
}

export function unavailableSelections(
  task: TaskType,
  selected: string[],
  ctx: PickerContext,
): UnavailableSelection[] {
  const found: UnavailableSelection[] = [];

  selected.forEach((id, index) => {
    const parsed = splitModelId(id);
    if (!parsed) return;
    if (!LISTED_PROVIDERS.includes(parsed.provider)) return;
    if (parsed.provider === 'assemblyai') return;
    if (parsed.provider === 'google' && parsed.model === GOOGLE_DEDICATED_TRANSCRIBER) return;

    const cache = ctx.cache[parsed.provider];
    if (!isTrustworthy(cache)) return;
    if (cache!.ids.includes(parsed.model)) return;
    if (!confirmedMissing(cache, id)) return;

    found.push({
      id,
      provider: parsed.provider,
      model: parsed.model,
      reason: cache!.everSeen.includes(parsed.model) ? 'deprecated' : 'not-found',
      position: index === 0 ? 'default' : index - 1,
      suggestion: suggestReplacement(task, parsed.provider, parsed.model, ctx),
    });
  });

  return found;
}

export function suggestReplacement(
  task: TaskType,
  provider: ProviderId,
  model: string,
  ctx: PickerContext,
): string | null {
  const dead = catalogEntry(provider, model);
  if (!dead) return null;

  const ranked = curatedRows({ ...ctx, task })
    .filter((row) => row.state === 'ready' && row.provider === provider && row.id !== model)
    .map((row) => {
      const entry = catalogEntry(provider, row.id)!;
      return {
        entry,
        shared: entry.tags.filter((tag) => dead.tags.includes(tag)).length,
        tierRank: entry.tier === 'premium' ? 0 : 1,
        order: CATALOG.indexOf(entry),
      };
    })
    .sort((a, b) => b.shared - a.shared || a.tierRank - b.tierRank || a.order - b.order);

  return ranked.length ? modelId(provider, ranked[0].entry.id) : null;
}

export function firstRunnable(chain: string[], ctx: PickerContext): string | null {
  for (const id of chain) {
    const row = rowForSelection(id, ctx);
    if (row?.state === 'ready') return id;
  }
  return null;
}

export function unavailableMessages(
  task: TaskType,
  defaultModel: string,
  fallbacks: string[],
  ctx: PickerContext,
): string[] {
  const chain = [defaultModel, ...fallbacks];
  const taskName = task === 'transcription' ? 'Transcription' : 'Clean-up';

  const unsupported = chain.flatMap((id, index) => {
    if (!unsupportedLocalPick(id, ctx)) return [];
    const name = modelDisplayLabel('local', splitModelId(id)!.model);
    if (index > 0) {
      return [`Fallback #${index} ${name} isn't available on this device and will be skipped.`];
    }
    const next = firstRunnable(fallbacks, ctx);
    if (!next) {
      return [`${name} isn't available on this device and no fallback is usable — ${taskName.toLowerCase()} will fail until you pick another model.`];
    }
    const parsed = splitModelId(next)!;
    return [`${name} isn't available on this device. ${taskName} will use ${qualifiedModelLabel(parsed.provider, parsed.model)} instead.`];
  });

  return [...unsupported, ...unavailableSelections(task, chain, ctx).map((entry) => {
    const name = modelDisplayLabel(entry.provider, entry.model);
    const provider = providerDisplayLabel(entry.provider);

    if (entry.reason === 'not-found') {
      return `Verenu can't find ${name} in ${provider}'s model list. Check the id or pick another model.`;
    }
    if (entry.position !== 'default') {
      return `Fallback #${entry.position + 1} ${name} is no longer offered by ${provider} and will be skipped.`;
    }

    const next = firstRunnable(fallbacks, ctx);
    if (!next) {
      return `${name} is no longer offered by ${provider} and no fallback is usable — ${taskName.toLowerCase()} will fail until you pick another model.`;
    }
    const parsed = splitModelId(next)!;
    const nextName = qualifiedModelLabel(parsed.provider, parsed.model);
    return `${name} is no longer offered by ${provider}. ${taskName} will use ${nextName} instead.`;
  })];
}

export { MISS_INTERVAL_MS, isTrustworthy };
