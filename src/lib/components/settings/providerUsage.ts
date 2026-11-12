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
};

const provider = (id: string | null | undefined) => (id ? splitModelId(id)?.provider ?? null : null);

function taskUse(target: string, main: string | null | undefined, fallbacks: string[] | null | undefined): TaskUse {
  if (provider(main) === target) return 'main';
  return (fallbacks ?? []).some((id) => provider(id) === target) ? 'backup' : null;
}

/** Clean-up that is turned off uses no provider, whatever model is still selected for it. */
export function providerUse(target: string, settings: UsageSettings): ProviderUse {
  return {
    transcription: taskUse(target, settings.transcription_default_model, settings.transcription_fallback_models),
    cleanup: settings.cleanup_enabled === false
      ? null
      : taskUse(target, settings.cleanup_default_model, settings.cleanup_fallback_models),
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
