import { invoke } from './tauri';
import type { ProviderId } from './settings';

export type CustomProvider = {
  id: `custom:${string}`;
  name: string;
  protocol: 'openai' | 'anthropic' | 'xai';
  base_url: string;
  requires_key: boolean;
  supports_transcription: boolean;
  supports_cleanup: boolean;
  auth_header: string | null;
  extra_headers: Record<string, string>;
  body_overrides: Record<string, unknown> | null;
  transcription_models: string[];
  cleanup_models: string[];
};

export const customProviderStore = $state<{ providers: CustomProvider[] }>({ providers: [] });
export function isCustomProviderId(id: string): id is `custom:${string}` {
  return /^custom:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id);
}
export function customProvider(id: ProviderId) {
  return customProviderStore.providers.find(p => p.id === id);
}
let nextRefresh = 0;
let appliedRefresh = 0;
export function beginCustomProviderRefresh() {
  const refresh = ++nextRefresh;
  return (providers: CustomProvider[]) => {
    if (refresh > appliedRefresh) {
      appliedRefresh = refresh;
      customProviderStore.providers = providers;
    }
  };
}
export async function refreshCustomProviders() {
  const apply = beginCustomProviderRefresh();
  const all = await invoke<{ custom_providers: CustomProvider[] }>('get_all_settings');
  apply(all.custom_providers ?? []);
}
