import { appleIntelligenceSupported, type AppleIntelligenceAvailability } from '../appleIntelligence.svelte';
import type { PresetTarget, RequiredLocalModel } from '../components/settings/modelPresets';
import type { setupDefaultModels } from './setupModelReadiness';

/** Apple Intelligence is cleanup-only; it is never a speech model. */
export const APPLE_CLEANUP_MODEL = 'apple-intelligence/system';

export type AppleCleanupOffer = {
  /** Whether the option is shown at all. Fails closed off supported Macs. */
  visible: boolean;
  /** Whether it can be chosen now. Supported-but-unready Macs see it, disabled. */
  selectable: boolean;
  /** Why it is not selectable yet; empty when ready. */
  reason: string;
};

export function appleCleanupOffer(status: AppleIntelligenceAvailability): AppleCleanupOffer {
  if (!appleIntelligenceSupported(status)) return { visible: false, selectable: false, reason: '' };
  if (status.available && status.state === 'available') return { visible: true, selectable: true, reason: '' };
  return { visible: true, selectable: false, reason: status.message };
}

/**
 * What the onboarding shows for Apple cleanup. A supported Mac gets the offer.
 * An explicit choice that is no longer offered gets a neutral recovery row that
 * can only clear it, never a selectable option. Nothing shows otherwise.
 */
export type AppleCleanupPanel = 'offer' | 'recovery' | 'hidden';

export function appleCleanupPanel(status: AppleIntelligenceAvailability, chosen: boolean): AppleCleanupPanel {
  if (appleCleanupOffer(status).visible) return 'offer';
  return chosen ? 'recovery' : 'hidden';
}

/**
 * The chosen preset with cleanup moved onto Apple Intelligence. Speech models,
 * dual transcription and every other choice stay; cleanup fallbacks are cleared
 * so a failure never silently falls back to a cloud or downloaded model, and no
 * cleanup model or engine download is required.
 */
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

/**
 * Local models a choice still needs downloaded, read through the Apple override.
 * Choosing a preset must start only these, so an Apple cleanup opt-in never
 * queues an unused cleanup model or engine download.
 */
export function missingLocalModels(
  target: PresetTarget | null | undefined,
  installed: Record<RequiredLocalModel['task'], string[]>,
  apple: boolean,
): RequiredLocalModel[] {
  return (applyAppleCleanup(target, apple)?.requiredLocalModels ?? [])
    .filter(model => !installed[model.task]?.includes(model.id));
}

/** Provider defaults (used when no preset was chosen) with cleanup on Apple Intelligence. */
export function applyAppleCleanupDefaults(
  defaults: ReturnType<typeof setupDefaultModels>,
  apple: boolean,
): ReturnType<typeof setupDefaultModels> {
  return apple ? { ...defaults, cleanupDefaultModel: APPLE_CLEANUP_MODEL } : defaults;
}

/**
 * A chosen Apple cleanup that is no longer ready must stop setup, not fall back
 * to the provider's cleanup model. Turning the choice off is the only way back.
 * Only cleanup that will actually run counts: with cleanup intensity Off the
 * unavailable engine is not a requirement.
 */
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
