// Auto-generated model recommendations for the Models tab.
//
// The simple (non-Advanced) Models view shows a short list of ready-to-use
// "presets" instead of the full per-provider model accordion. Which presets we
// offer considers credentials, language, capacity, catalog retirement, and
// recent process-local performance. Recommendations never change settings.

import { invoke } from '../../tauri';
import type { ProviderId } from '../../settings';
import { isAndroid } from '../../platform';
import { getTranscriptionLanguageLabel } from '../../transcriptionLanguages';
import { getLanguageSupport } from '../../transcriptionLanguageSupport';
import { isTrustworthy, type ModelCatalogCache } from '../../modelCatalogStore.svelte';
import type { LocalSttModelInfo } from '../../tauri';
import type { CustomProvider } from '../../customProviders.svelte';
import {
  GROQ_QWEN_3_8_27B_MODEL,
  modelId,
  recommendedModels,
  CATALOG,
  providerSections,
  splitModelId,
  type TaskType,
  type UiProviderId,
} from './models';

export type Hardware = {
  /** Native platform capability, independent of the window width or user-agent. */
  isAndroid?: boolean;
  totalRamMb: number;
  freeRamMb: number;
  gpus: { vramTotalMb: number; vramUsedMb: number }[];
  /**
   * True when the backend read failed (or an older backend lacks the command).
   * In that case we assume a capable machine and offer every preset rather than
   * hiding local options behind a phantom "not enough RAM".
   */
  unknown: boolean;
};

export type RequiredLocalModel = { task: TaskType; id: string; sizeMb: number };

export type PresetTarget = {
  transcriptionDefaultModel: string;
  cleanupEnabled: boolean;
  cleanupDefaultModel: string | null;
  dualTranscription: boolean;
  transcriptionFallbacks: string[];
  cleanupFallbacks: string[];
  /** Local model ids that must be on disk before this preset can run. */
  requiredLocalModels: RequiredLocalModel[];
};

export type Preset = {
  id: string;
  /** 'preset' = selectable config; 'add-key' = an inert prompt to add a key. */
  kind: 'preset' | 'add-key';
  name: string;
  tagline: string;
  /** 0 = maximum accuracy … 1 = maximum efficiency; positions the bar marker. */
  position: number;
  offline: boolean;
  target: PresetTarget | null;
};

/** What the live settings currently look like — compared against preset targets. */
export type ActiveConfig = {
  transcriptionDefaultModel: string;
  cleanupEnabled: boolean;
  cleanupDefaultModel: string;
  dualTranscription: boolean;
  transcriptionFallbacks: string[];
  cleanupFallbacks: string[];
};

export type ModelPerformance = {
  task: TaskType;
  id: string;
  samples: number;
  failures: number;
  latency_ms: number;
  updated_at_ms: number;
};

export type PresetOptions = {
  language?: string;
  localModels?: LocalSttModelInfo[];
  installedLocal?: { transcription: string[]; cleanup: string[] };
  localCleanupReady?: boolean;
  /** Offer a speech-only local choice for the setup wizard. */
  includeTranscriptionOnly?: boolean;
  /** Keep the wizard's local provider choice on local models even when cloud keys exist. */
  localOnly?: boolean;
  cache?: ModelCatalogCache;
  performance?: ModelPerformance[];
  customProviders?: CustomProvider[];
};

export function modelLabel(id: string): string {
  const parsed = splitModelId(id);
  const label = CATALOG.find(entry => entry.provider === parsed?.provider && entry.id === parsed.model)?.label ?? parsed?.model ?? id;
  const provider = parsed?.provider === 'local' ? 'Local' : providerSections.find(provider => provider.id === parsed?.provider)?.label ?? parsed?.provider;
  return provider ? `${provider}: ${label}` : label;
}

export function supportsLocalLanguage(id: string, options: PresetOptions): boolean {
  const model = options.localModels?.find(model => model.id === id);
  // An auxiliary download cannot transcribe, regardless of language.
  if (id === 'fluid-english-booster' || model?.engine_type === 'ctc_booster') return false;
  if (!options.language || options.language === 'auto') return true;
  const languages = model?.supported_languages;
  // Whisper, Parakeet Ultra and Apple Speech report a wildcard instead of
  // language names. The frontend map holds the dropdown subset for Whisper and
  // Ultra; Apple Speech has no entry on purpose (its languages are whatever
  // macOS offers), so it falls through to 'all' and the real locale check
  // runs when dictation starts.
  const wildcard = languages?.some(language => /^(multilingual|system languages)$/i.test(language));
  if (!languages || wildcard) {
    const support = getLanguageSupport('local', id);
    return support === 'all' || support.some(code => code === options.language);
  }
  return languages.some(language => language.toLowerCase() === getTranscriptionLanguageLabel(options.language!).toLowerCase());
}

/** Absence during a provider outage must never retire a recommendation. */
function usableRecommendation(id: string, task: TaskType, options: PresetOptions): boolean {
  const parsed = splitModelId(id);
  if (!parsed) return false;
  if (task === 'transcription' && options.language && options.language !== 'auto') {
    const support = getLanguageSupport(parsed.provider, parsed.model);
    if (support !== 'all' && !support.some(code => code === options.language)) return false;
  }
  const cache = options.cache?.[parsed.provider];
  const metadata = cache?.metadata?.[parsed.model];
  if (metadata && !metadata.tasks.includes(task)) return false;
  return !(isTrustworthy(cache) && !cache!.ids.includes(parsed.model) && (cache!.missing[id]?.count ?? 0) >= 2);
}

function measuredOrder(ids: string[], task: TaskType, options: PresetOptions): string[] {
  const measurement = (id: string) => options.performance?.find(sample => sample.id === id && sample.task === task && sample.samples >= 3 && Date.now() - sample.updated_at_ms < 7 * 86400000);
  const unhealthy = (id: string) => { const sample = measurement(id); return !!sample && sample.failures * 2 >= sample.samples; };
  const ordered = [...new Set(ids)].sort((a, b) => Number(unhealthy(a)) - Number(unhealthy(b)));
  // Reorder only measured positions in each reliability group. Unknown
  // candidates keep their curated positions, so the ordering is transitive.
  for (const failed of [false, true]) {
    const measured = ordered.map((id, index) => ({ id, index, sample: measurement(id) }))
      .filter(row => row.sample && unhealthy(row.id) === failed);
    const sorted = [...measured].sort((a, b) => a.sample!.latency_ms - b.sample!.latency_ms);
    measured.forEach((row, index) => { ordered[row.index] = sorted[index].id; });
  }
  return ordered;
}

// ── Hardware ──────────────────────────────────────────────────────────────

// A generous stand-in used when the RAM read fails — "assume capable" so the
// picker never wrongly hides local presets. 16 GB total / 12 GB free clears
// every threshold below.
const CAPABLE_DEFAULT: Hardware = { totalRamMb: 16384, freeRamMb: 12288, gpus: [], unknown: true };

type RawHardware = {
  is_android?: boolean;
  total_ram_mb?: number;
  free_ram_mb?: number;
  gpus?: { vram_total_mb?: number; vram_used_mb?: number }[];
};

export async function getHardware(): Promise<Hardware> {
  try {
    const raw = await invoke<RawHardware>('get_hardware_capabilities');
    // total_ram_mb === 0 is the backend's "read failed" sentinel — treat it the
    // same as a thrown error and fall back to the capable default.
    if (!raw || !raw.total_ram_mb) return { ...CAPABLE_DEFAULT, isAndroid: raw?.is_android ?? false };
    return {
      isAndroid: raw.is_android ?? false,
      totalRamMb: raw.total_ram_mb,
      freeRamMb: raw.free_ram_mb ?? 0,
      gpus: (raw.gpus ?? []).map((gpu) => ({
        vramTotalMb: gpu.vram_total_mb ?? 0,
        vramUsedMb: gpu.vram_used_mb ?? 0,
      })),
      unknown: false,
    };
  } catch {
    // The platform probe is only a fallback when native hardware IPC fails.
    // Prefer the backend's is_android value whenever the command succeeds.
    return { ...CAPABLE_DEFAULT, isAndroid };
  }
}

// ── RAM viability ─────────────────────────────────────────────────────────

// Total (not free) RAM is the capacity signal for *which presets to offer* —
// idle models get unloaded and the user can close other apps, so gating on the
// current free figure would wrongly hide "Most accurate" on a big machine that
// happens to be busy right now. Runtime memory pressure is handled separately
// on the backend. Headroom covers the OS, the app, and the target editor app.
const OS_HEADROOM_MB = 3000;
const RAM_FACTOR = 1.3;

function ramNeededMb(sizesMb: number[]): number {
  const total = sizesMb.reduce((sum, size) => sum + size, 0);
  return Math.round(total * RAM_FACTOR) + OS_HEADROOM_MB;
}

export function fitsHardware(hardware: Hardware, sizesMb: number[]): boolean {
  return hardware.unknown || hardware.totalRamMb >= ramNeededMb(sizesMb);
}

// ── Local model catalog (mirrors the Rust size_mb catalogs) ───────────────
// Only the ids used by presets. Gemma is intentionally excluded — its curated
// GGUFs are flagged not-recommended in the backend catalog (tokenizer issues).

const STT_PARAKEET_V3 = { id: 'parakeet-v3', sizeMb: 456 };
const STT_MOONSHINE_TINY = { id: 'moonshine-tiny', sizeMb: 31 };
const STT_COHERE = { id: 'cohere', sizeMb: 1708 };

const LLM_QWEN_1_5B = { id: 'qwen2.5-1.5b-instruct', sizeMb: 1080 };
const LLM_QWEN_3B = { id: 'qwen2.5-3b-instruct', sizeMb: 1960 };
const LLM_QWEN_7B = { id: 'qwen2.5-7b-instruct', sizeMb: 4680 };
const LLM_QWEN_0_5B = { id: 'qwen2.5-0.5b-instruct', sizeMb: 430 };

type LocalTier = {
  key: string;
  name: string;
  tagline: string;
  position: number;
  stt: { id: string; sizeMb: number };
  llm: { id: string; sizeMb: number } | null;
};

// Ordered most-efficient → most-accurate.
const LOCAL_TIERS: LocalTier[] = [
  {
    key: 'fastest',
    name: 'Fastest',
    tagline: 'Fast and light. Runs entirely on your device, private and offline.',
    position: 0.8,
    stt: STT_PARAKEET_V3,
    llm: LLM_QWEN_1_5B,
  },
  {
    key: 'balanced',
    name: 'Balanced',
    tagline: 'A good mix of speed and accuracy. Runs entirely on your device, private and offline.',
    position: 0.5,
    stt: STT_PARAKEET_V3,
    llm: LLM_QWEN_3B,
  },
  {
    key: 'accurate',
    name: 'Quality',
    tagline: 'Stronger speech and cleanup models. Private and offline.',
    position: 0.2,
    stt: STT_COHERE,
    llm: LLM_QWEN_7B,
  },
];

// Phones get one local pair: the 0.5B cleanup model. The 1.5B model made cleanup
// take far too long on real phones, so it is no longer offered as a preset (it
// stays available under Advanced Models).
const ANDROID_LOCAL_TIERS: LocalTier[] = [
  {
    key: 'fastest', name: 'Fastest',
    tagline: 'Small English speech and cleanup models. Private and offline.',
    position: 0.85, stt: STT_MOONSHINE_TINY, llm: LLM_QWEN_0_5B,
  },
];

function localTiers(hardware: Hardware): LocalTier[] {
  // Unknown phone RAM gets the smallest pair rather than desktop-sized defaults.
  if (hardware.isAndroid) return ANDROID_LOCAL_TIERS;
  return LOCAL_TIERS;
}

function localTierSizes(tier: LocalTier): number[] {
  return tier.llm ? [tier.stt.sizeMb, tier.llm.sizeMb] : [tier.stt.sizeMb];
}

function localTierTarget(tier: LocalTier): PresetTarget {
  const required: RequiredLocalModel[] = [{ task: 'transcription', id: tier.stt.id, sizeMb: tier.stt.sizeMb }];
  if (tier.llm) required.push({ task: 'cleanup', id: tier.llm.id, sizeMb: tier.llm.sizeMb });
  return {
    transcriptionDefaultModel: modelId('local', tier.stt.id),
    cleanupEnabled: tier.llm !== null,
    cleanupDefaultModel: tier.llm ? modelId('local', tier.llm.id) : null,
    dualTranscription: false,
    transcriptionFallbacks: [],
    cleanupFallbacks: [],
    requiredLocalModels: required,
  };
}

function localTierPreset(tier: LocalTier, idPrefix: string): Preset {
  return {
    id: `${idPrefix}-${tier.key}`,
    kind: 'preset',
    name: tier.name,
    tagline: tier.tagline,
    position: tier.position,
    offline: true,
    target: localTierTarget(tier),
  };
}

// The floor: transcription with no cleanup, for machines too small for a local
// LLM (or with no key to run cloud cleanup). Uses the lightest STT that fits.
function transcriptionOnlyPreset(hardware: Hardware): Preset {
  const stt = !hardware.isAndroid && fitsHardware(hardware, [STT_PARAKEET_V3.sizeMb]) ? STT_PARAKEET_V3 : STT_MOONSHINE_TINY;
  return {
    id: 'local-transcription-only',
    kind: 'preset',
    name: 'Transcription only',
    tagline: 'Local AI for speech-to-text. Runs entirely on your device, private and offline.',
    position: 0.9,
    offline: true,
    target: {
      transcriptionDefaultModel: modelId('local', stt.id),
      cleanupEnabled: false,
      cleanupDefaultModel: null,
      dualTranscription: false,
      transcriptionFallbacks: [],
      cleanupFallbacks: [],
      requiredLocalModels: [{ task: 'transcription', id: stt.id, sizeMb: stt.sizeMb }],
    },
  };
}

// ── Cloud provider selection ──────────────────────────────────────────────

type KeyStatus = Record<Exclude<ProviderId, 'apple-intelligence'>, boolean>;

const CLOUD_PROVIDERS: UiProviderId[] = ['groq', 'openai', 'google', 'assemblyai', 'openrouter', 'xai'];

function hasCloudKey(status: KeyStatus): boolean {
  return CLOUD_PROVIDERS.some((provider) => status[provider]);
}

function transcriptionModelFor(provider: UiProviderId, tier: 'standard' | 'premium'): string {
  // Safe: every cloud provider has a recommendedModels.transcription entry.
  return modelId(provider, recommendedModels.transcription[provider]![tier]);
}

function cleanupModelFor(provider: UiProviderId | undefined, tier: 'standard' | 'premium'): string | null {
  if (!provider) return null;
  const entry = recommendedModels.cleanup[provider];
  if (!entry) return null;
  // Groq's former standard cleanup model is being retired. Keep the catalog
  // entry for recognizing old selections, but never put it into a new preset.
  if (provider === 'groq' && tier === 'standard') {
    return modelId(provider, GROQ_QWEN_3_8_27B_MODEL);
  }
  return modelId(provider, entry[tier]);
}

// ── Public: build the preset list ─────────────────────────────────────────

/**
 * The one local engine that stays offered where the blanket on-device gate is
 * closed (Intel Macs): Apple Speech, which ships with macOS and needs no Verenu
 * download. It must be listed by the backend as a system-managed engine, so an
 * unloaded, empty, or failed list stays conservative. Eligibility is not
 * permission or locale-asset readiness; those are checked when dictating.
 * Whisper, ONNX, GGML, Fluid and cleanup models are never admitted here.
 */
export function eligibleSystemSpeechId(options: PresetOptions): string | null {
  const model = options.localModels?.find(m => m.engine_type === 'apple_speech' && m.install_kind === 'system_managed');
  return model && supportsLocalLanguage(model.id, options) ? model.id : null;
}

function systemSpeechPreset(id: string): Preset {
  return {
    id: 'local-transcription-only',
    kind: 'preset',
    name: 'Transcription only',
    tagline: 'Apple Speech on this Mac. No model download; macOS may ask for speech permission.',
    position: 0.9,
    offline: true,
    target: {
      transcriptionDefaultModel: modelId('local', id),
      cleanupEnabled: false,
      cleanupDefaultModel: null,
      dualTranscription: false,
      transcriptionFallbacks: [],
      cleanupFallbacks: [],
      requiredLocalModels: [],
    },
  };
}

export function buildPresets(status: KeyStatus, hardware: Hardware, localSupported: boolean, options: PresetOptions = {}): Preset[] {
  // With the blanket gate closed only eligible system speech survives: as a
  // transcription-only preset and as an installed cloud fallback, never cleanup.
  const systemSpeech = localSupported ? null : eligibleSystemSpeechId(options);
  const gatedOptions: PresetOptions = {
    ...options,
    installedLocal: systemSpeech && options.installedLocal?.transcription.includes(systemSpeech)
      ? { transcription: [systemSpeech], cleanup: [] }
      : undefined,
  };
  if (options.localOnly) {
    if (!localSupported) return systemSpeech ? [systemSpeechPreset(systemSpeech)] : [addKeyPreset()];
    const local = buildLocalOnlyPresets(hardware, options);
    if (!options.includeTranscriptionOnly || local.some(preset => preset.id === 'local-transcription-only')) return local;
    return [transcriptionOnlyPreset(hardware), ...local];
  }
  if (hasCloudKey(status) || options.customProviders?.some(provider => provider.supports_transcription && (!provider.requires_key || status[provider.id]))) {
    return [
      ...buildCloudPresets(status, localSupported ? options : gatedOptions),
      ...(localSupported ? buildLocalOnlyPresets(hardware, options) : systemSpeech ? [systemSpeechPreset(systemSpeech)] : []),
    ];
  }
  if (localSupported) {
    return buildLocalOnlyPresets(hardware, options);
  }
  // No keys and local inference unavailable (e.g. Intel Mac): eligible system
  // speech, otherwise the only path forward is adding an API key.
  return systemSpeech ? [systemSpeechPreset(systemSpeech)] : [addKeyPreset()];
}

const TRANSCRIPTION_ORDER: UiProviderId[] = ['groq', 'openai', 'google', 'assemblyai', 'openrouter', 'xai'];
const CLEANUP_FAST_ORDER: UiProviderId[] = ['groq', 'openai', 'google', 'openrouter', 'xai'];
const CLEANUP_ACCURATE_ORDER: UiProviderId[] = ['openai', 'google', 'groq', 'openrouter', 'xai'];

// Every other keyed transcription provider's model at the same tier, in
// preference order — so a preset degrades across providers, not just across the
// primary provider's own models. Excludes the primary (already the default).
function cloudCandidates(status: KeyStatus, task: TaskType, tier: 'standard' | 'premium', options: PresetOptions): string[] {
  const order = task === 'transcription' ? TRANSCRIPTION_ORDER : tier === 'premium' ? CLEANUP_ACCURATE_ORDER : CLEANUP_FAST_ORDER;
  const ids = order.filter(provider => status[provider]).flatMap(provider => {
    const preferred = recommendedModels[task][provider]
      ? task === 'transcription' ? transcriptionModelFor(provider, tier) : cleanupModelFor(provider, tier)
      : null;
    // A retired tier can use a compatible curated sibling. Discovery alone
    // never promotes a new, unevaluated model into an automatic default.
    const siblings = CATALOG.filter(entry => entry.provider === provider && entry.tasks.includes(task))
      .filter(entry => entry.id !== recommendedModels.cleanup.groq?.standard || task !== 'cleanup')
      .map(entry => modelId(provider, entry.id));
    return [...new Set([...(preferred ? [preferred] : []), ...siblings])]
      .filter(id => usableRecommendation(id, task, options)).slice(0, 1);
  });
  // Custom models are explicitly configured by the user. Preserve their order;
  // they are not promoted based on an unverified discovery response.
  const custom = (options.customProviders ?? []).filter(provider => !provider.requires_key || status[provider.id])
    .filter(provider => task === 'transcription' ? provider.supports_transcription : provider.supports_cleanup)
    .flatMap(provider => (task === 'transcription' ? provider.transcription_models : provider.cleanup_models).map(id => modelId(provider.id, id)));
  return [...measuredOrder(ids, task, options), ...custom];
}

export function installedLocalFallbacks(task: TaskType, options: PresetOptions): string[] {
  if (task === 'cleanup' && options.localCleanupReady === false) return [];
  const ids = (options.installedLocal?.[task] ?? []).filter(id => {
    if (task === 'transcription') return supportsLocalLanguage(id, options);
    return CATALOG.some(entry => entry.provider === 'local' && entry.id === id && entry.tasks.includes(task));
  });
  // Start with a modest model; installed alternatives remain available for
  // dual comparison or recovery. Measurements can reorder these later.
  return measuredOrder(ids.map(id => modelId('local', id)), task, options);
}

function buildCloudPresets(status: KeyStatus, options: PresetOptions): Preset[] {
  const localSpeech = installedLocalFallbacks('transcription', options);
  const localCleanup = installedLocalFallbacks('cleanup', options);
  return (['fastest', 'balanced', 'accurate'] as const).flatMap((key): Preset[] => {
    const tier = key === 'fastest' ? 'standard' : 'premium';
    const speech = cloudCandidates(status, 'transcription', tier, options);
    const cleanup = cloudCandidates(status, 'cleanup', key === 'accurate' ? 'premium' : 'standard', options);
    if (!speech.length) return [];
    // Quality always obtains a second transcript. Prefer a different model
    // from the primary provider, then continue through the cloud chain.
    const secondary = key === 'accurate'
      ? cloudCandidates(status, 'transcription', 'standard', options).filter(id => id !== speech[0])
      : [];
    const transcriptionFallbacks = [...new Set([...secondary.slice(0, 1), ...speech.slice(1), ...secondary.slice(1), ...localSpeech])];
    return [{
      id: `cloud-${key}`, kind: 'preset',
      name: key === 'fastest' ? 'Fastest' : key === 'balanced' ? 'Balanced' : 'Quality',
      tagline: key === 'accurate' ? 'Always compares two speech models to reduce hallucinated additions.' : key === 'fastest' ? 'Lighter models for short response times.' : 'Stronger speech recognition with lightweight cleanup.',
      position: key === 'fastest' ? 0.88 : key === 'balanced' ? 0.5 : 0.12,
      offline: false,
      target: cloudTarget({
        transcriptionDefaultModel: speech[0],
        transcriptionFallbacks,
        cleanupDefaultModel: cleanup[0] ?? localCleanup[0] ?? null,
        cleanupFallbacks: [...cleanup.slice(1), ...localCleanup].filter(id => id !== (cleanup[0] ?? localCleanup[0])),
        dualTranscription: key === 'accurate',
      }),
    }];
  });
}

function cloudTarget(opts: {
  transcriptionDefaultModel: string;
  transcriptionFallbacks: string[];
  cleanupDefaultModel: string | null;
  cleanupFallbacks: string[];
  dualTranscription: boolean;
}): PresetTarget {
  const cleanupEnabled = opts.cleanupDefaultModel !== null;
  return {
    transcriptionDefaultModel: opts.transcriptionDefaultModel,
    cleanupEnabled,
    cleanupDefaultModel: opts.cleanupDefaultModel,
    dualTranscription: opts.dualTranscription,
    transcriptionFallbacks: opts.transcriptionFallbacks,
    cleanupFallbacks: cleanupEnabled ? opts.cleanupFallbacks : [],
    requiredLocalModels: [],
  };
}

function buildLocalOnlyPresets(hardware: Hardware, options: PresetOptions): Preset[] {
  const viable = localTiers(hardware).filter((tier) => fitsHardware(hardware, localTierSizes(tier)) && supportsLocalLanguage(tier.stt.id, options));
  if (viable.length === 0) {
    const installed = installedLocalFallbacks('transcription', options)[0];
    if (installed) return [{ ...transcriptionOnlyPreset(hardware), target: { ...transcriptionOnlyPreset(hardware).target!, transcriptionDefaultModel: installed, requiredLocalModels: [] } }];
    const floor = transcriptionOnlyPreset(hardware);
    return supportsLocalLanguage(floor.target!.requiredLocalModels[0].id, options) ? [floor] : [];
  }
  return viable.map(original => {
    let tier = original;
    if (tier.key === 'fastest') {
      const measured = (options.localModels ?? []).filter(model => model.is_downloaded && supportsLocalLanguage(model.id, options) && fitsHardware(hardware, [model.size_mb, ...(tier.llm ? [tier.llm.sizeMb] : [])]))
        .map(model => ({ model, sample: options.performance?.find(sample => sample.task === 'transcription' && sample.id === modelId('local', model.id) && sample.samples >= 3 && sample.failures === 0 && Date.now() - sample.updated_at_ms < 7 * 86400000) }))
        .filter(row => row.sample && row.sample.latency_ms <= 10_000)
        .sort((a, b) => a.sample!.latency_ms - b.sample!.latency_ms)[0];
      if (measured) tier = { ...tier, stt: { id: measured.model.id, sizeMb: measured.model.size_mb } };
    }
    const preset = localTierPreset(tier, 'local');
    if (tier.key === 'accurate') {
      preset.target!.dualTranscription = true;
      if (supportsLocalLanguage(STT_PARAKEET_V3.id, options) && fitsHardware(hardware, [...localTierSizes(tier), STT_PARAKEET_V3.sizeMb])) {
        preset.target!.transcriptionFallbacks = [modelId('local', STT_PARAKEET_V3.id)];
        preset.target!.requiredLocalModels.push({ task: 'transcription', ...STT_PARAKEET_V3 });
      }
    }
    const otherSpeech = installedLocalFallbacks('transcription', options).filter(id => id !== preset.target!.transcriptionDefaultModel);
    preset.target!.transcriptionFallbacks = [...new Set([...preset.target!.transcriptionFallbacks, ...otherSpeech])];
    preset.target!.cleanupFallbacks = installedLocalFallbacks('cleanup', options).filter(id => id !== preset.target!.cleanupDefaultModel);
    return preset;
  });
}

function addKeyPreset(): Preset {
  return {
    id: 'add-key',
    kind: 'add-key',
    name: 'Add an API key',
    tagline: 'Add a Groq, OpenAI, or Gemini key to start dictating. Groq is free and recommended.',
    position: 0.5,
    offline: false,
    target: null,
  };
}

// ── Public: match live settings to a preset ───────────────────────────────

function sameFallbacks(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((value, index) => value === b[index]);
}

export function matchActivePreset(presets: Preset[], current: ActiveConfig): string | null {
  for (const preset of presets) {
    const target = preset.target;
    if (!target) continue;
    if (target.transcriptionDefaultModel !== current.transcriptionDefaultModel) continue;
    if (target.dualTranscription !== current.dualTranscription) continue;
    if (target.cleanupEnabled !== current.cleanupEnabled) continue;
    if (target.cleanupEnabled && target.cleanupDefaultModel !== current.cleanupDefaultModel) continue;
    const relevant = (ids: string[]) => preset.offline ? ids : ids.filter(id => !id.startsWith('local/'));
    if (!sameFallbacks(relevant(target.transcriptionFallbacks), relevant(current.transcriptionFallbacks))) continue;
    if (target.cleanupEnabled && !sameFallbacks(relevant(target.cleanupFallbacks), relevant(current.cleanupFallbacks))) continue;
    return preset.id;
  }
  return null;
}
