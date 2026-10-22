import { appleIntelligenceSupported, type AppleIntelligenceAvailability } from '../appleIntelligence.svelte';
import type { PresetTarget, RequiredLocalModel } from '../components/settings/modelPresets';
import type { setupDefaultModels } from './setupModelReadiness';

export const APPLE_CLEANUP_MODEL = 'apple-intelligence/system';

export type AppleCleanupOffer = {
  visible: boolean;
  selectable: boolean;
  reason: string;
};

export function appleCleanupOffer(status: AppleIntelligenceAvailability): AppleCleanupOffer {
  if (!appleIntelligenceSupported(status)) return { visible: false, selectable: false, reason: '' };
  if (status.available && status.state === 'available') return { visible: true, selectable: true, reason: '' };
  return { visible: true, selectable: false, reason: status.message };
}

export type AppleCleanupPanel = 'offer' | 'recovery' | 'hidden';

export function appleCleanupPanel(status: AppleIntelligenceAvailability, chosen: boolean): AppleCleanupPanel {
  if (appleCleanupOffer(status).visible) return 'offer';
  return chosen ? 'recovery' : 'hidden';
}

export function applyAppleCleanup(target: PresetTarget | null | undefined, apple: boolean): PresetTarget | null {
  if (!target) return null;
  if (!apple) return target;
  return {
    ...target,
    cleanupEnabled: true,
    cleanupDefaultModel: APPLE_CLEANUP_MODEL,
    cleanupFallbacks: [],
    requiredLocalModels: target.requiredLocalModels.filter(model => model.task !== 'cleanup'),
  };
}

export function missingLocalModels(
  target: PresetTarget | null | undefined,
  installed: Record<RequiredLocalModel['task'], string[]>,
  apple: boolean,
): RequiredLocalModel[] {
  return (applyAppleCleanup(target, apple)?.requiredLocalModels ?? [])
    .filter(model => !installed[model.task]?.includes(model.id));
}

export function applyAppleCleanupDefaults(
  defaults: ReturnType<typeof setupDefaultModels>,
  apple: boolean,
): ReturnType<typeof setupDefaultModels> {
  return apple ? { ...defaults, cleanupDefaultModel: APPLE_CLEANUP_MODEL } : defaults;
}

export function appleCleanupReadiness(
  chosen: boolean,
  cleanupEnabled: boolean,
  status: AppleIntelligenceAvailability,
): { ready: boolean; message: string } {
  if (!chosen || !cleanupEnabled || appleCleanupOffer(status).selectable) return { ready: true, message: '' };
  const reason = appleCleanupOffer(status).visible && status.message ? ` ${status.message}` : '';
  return {
    ready: false,
    message: `Apple Intelligence cleanup is selected but is not ready on this Mac.${reason} Fix it, or turn it off in Models to use the other cleanup.`,
  };
}
