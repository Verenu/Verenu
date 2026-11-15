import { splitModelId } from './models';

/** How one task uses a provider: its main model, a backup in the fallback chain, or not at all. */
export type TaskUse = 'main' | 'backup' | null;
export type ProviderUse = { transcription: TaskUse; cleanup: TaskUse };

export type UsageSettings = {
  transcription_default_model?: string | null;
  transcription_fallback_models?: string[] | null;
  cleanup_default_model?: string | null;
  cleanup_fallback_models?: string[] | null;
  cleanup_enabled?: boolean | null;
  dual_transcription_enabled?: boolean | null;
  model_selection_mode?: string | null;
  /** Older settings name the model and provider separately; the backend still reads them when the newer field is unset. */
  transcription_model?: string | null;
  transcription_provider?: string | null;
  cleanup_model?: string | null;
  cleanup_provider?: string | null;
};

/** The backend lowercases and trims the provider half of an id before it accepts it. */
function provider(id: string | null | undefined): string | null {
  const slash = id?.indexOf('/') ?? -1;
  if (!id || slash <= 0) return null;
  return splitModelId(`${id.slice(0, slash).trim().toLowerCase()}${id.slice(slash)}`)?.provider ?? null;
}

/**
 * The provider of a task's main model, in the order `load_pipeline_config` resolves it: the newer field, then the
 * older model field, which defaults to a Groq model when unset, then the saved provider when that older value has
 * no provider of its own.
 */
function mainProvider(current: string | null | undefined, legacyModel: string | null | undefined, legacyProvider: string | null | undefined): string {
  return provider(current) ?? (legacyModel ? provider(legacyModel) ?? (legacyProvider || 'groq') : 'groq');
}

const onDevice = (id: string) => id === 'local' || id === 'apple-intelligence';

/**
 * How many leading cloud providers of a chain can run first. A manual selection runs the main model, plus the first
 * fallback when two speech models are compared, and so does the Quality setup for speech. Every other automatic
 * mode reorders cloud models by measured speed, so any of them can run first.
 */
function running(task: 'transcription' | 'cleanup', settings: UsageSettings, savedOrder: boolean, cloud: number): number {
  const compared = task === 'transcription' && settings.dual_transcription_enabled === true ? 2 : 1;
  // Quality keeps its first two speech models in their saved order; the second runs only when two are compared.
  if (savedOrder || (task === 'transcription' && settings.model_selection_mode === 'quality')) return compared;
  return cloud;
}

function taskUse(target: string, task: 'transcription' | 'cleanup', settings: UsageSettings): TaskUse {
  const main = task === 'transcription'
    ? mainProvider(settings.transcription_default_model, settings.transcription_model, settings.transcription_provider)
    : mainProvider(settings.cleanup_default_model, settings.cleanup_model, settings.cleanup_provider);
  const fallbacks = (task === 'transcription' ? settings.transcription_fallback_models : settings.cleanup_fallback_models) ?? [];
  const chain = [main, ...fallbacks.map(provider).filter((id): id is string => id !== null)];
  // A selected Apple Intelligence clean-up model stays first in every mode, so nothing after it is reordered ahead.
  const savedOrder = (settings.model_selection_mode ?? 'manual') === 'manual' || (task === 'cleanup' && main === 'apple-intelligence');
  // Automatic modes try cloud models before on-device ones; otherwise the chain runs in the saved order.
  const ordered = savedOrder ? chain : chain.filter((id) => !onDevice(id));
  const position = ordered.indexOf(target);
  if (position === -1) return chain.includes(target) ? 'backup' : null;
  return position < running(task, settings, savedOrder, ordered.length) ? 'main' : 'backup';
}

/** Clean-up that is turned off uses no provider, whatever model is still selected for it. */
export function providerUse(target: string, settings: UsageSettings): ProviderUse {
  return {
    transcription: taskUse(target, 'transcription', settings),
    cleanup: settings.cleanup_enabled === false ? null : taskUse(target, 'cleanup', settings),
  };
}

const TASK_NAMES = { transcription: 'transcription', cleanup: 'clean-up' } as const;

export function usageLabel(use: ProviderUse): string {
  const tasks = ['transcription', 'cleanup'] as const;
  const main = tasks.filter((task) => use[task] === 'main').map((task) => TASK_NAMES[task]);
  const backup = tasks.filter((task) => use[task] === 'backup').map((task) => TASK_NAMES[task]);
  if (main.length && backup.length) return `Runs ${main.join(' and ')}, backup for ${backup.join(' and ')}`;
  if (main.length) return `Runs ${main.join(' and ')}`;
  if (backup.length) return `Backup for ${backup.join(' and ')}`;
  return 'Not in use';
}

export function usageLevel(use: ProviderUse): 'main' | 'backup' | 'idle' {
  if (use.transcription === 'main' || use.cleanup === 'main') return 'main';
  return use.transcription === 'backup' || use.cleanup === 'backup' ? 'backup' : 'idle';
}
