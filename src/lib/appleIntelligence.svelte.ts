import { invoke } from './tauri';

export type AppleIntelligenceAvailability = {
  state: string;
  available: boolean;
  message: string;
};

export const appleIntelligence = $state<{ status: AppleIntelligenceAvailability }>({
  status: { state: 'checking', available: false, message: 'Checking Apple Intelligence availability...' },
});

let pending: Promise<AppleIntelligenceAvailability> | undefined;
export function refreshAppleIntelligence(): Promise<AppleIntelligenceAvailability> {
  return pending ??= invoke<AppleIntelligenceAvailability>('get_apple_intelligence_availability')
    .catch(() => ({ state: 'unavailable', available: false, message: 'Could not check Apple Intelligence. Try refreshing models.' }))
    .then(status => { appleIntelligence.status = status; return status; })
    .finally(() => { pending = undefined; });
}
