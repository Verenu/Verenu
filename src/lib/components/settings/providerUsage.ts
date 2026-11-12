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
  /** Older settings name the model and provider separately; the backend still reads them when the newer field is unset. */
  transcription_model?: string | null;
  transcription_provider?: string | null;
  cleanup_model?: string | null;
  cleanup_provider?: string | null;
};

const provider = (id: string | null | undefined) => (id ? splitModelId(id)?.provider ?? null : null);

/** Same order as the backend's pipeline config: the newer field, the older model field, then the saved provider, then Groq. */
function mainProvider(current: string | null | undefined, legacyModel: string | null | undefined, legacyProvider: string | null | undefined): string {
  return provider(current) ?? provider(legacyModel) ?? (legacyProvider || 'groq');
}

function taskUse(target: string, main: string, fallbacks: string[] | null | undefined, alsoRuns = 0): TaskUse {
  const chain = fallbacks ?? [];
  if (main === target || chain.slice(0, alsoRuns).some((id) => provider(id) === target)) return 'main';
  return chain.some((id) => provider(id) === target) ? 'backup' : null;
}

/**
 * Clean-up that is turned off uses no provider, whatever model is still selected for it.
 * Dual transcription runs the first fallback beside the main model, so that provider runs transcription too.
 */
export function providerUse(target: string, settings: UsageSettings): ProviderUse {
  return {
    transcription: taskUse(
      target,
      mainProvider(settings.transcription_default_model, settings.transcription_model, settings.transcription_provider),
      settings.transcription_fallback_models,
      settings.dual_transcription_enabled === true ? 1 : 0,
    ),
    cleanup: settings.cleanup_enabled === false
      ? null
      : taskUse(target, mainProvider(settings.cleanup_default_model, settings.cleanup_model, settings.cleanup_provider), settings.cleanup_fallback_models),
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
