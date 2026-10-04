import type { CustomProvider } from './customProviders.svelte';

/**
 * Starting points for the custom provider editor. These are conveniences, not
 * endorsements: Verenu does not test or officially support these vendors, and
 * model IDs change often, so every value stays editable before saving.
 */
export type PresetGroup = 'cloud' | 'gateway' | 'local' | 'advanced';

/** A second wire format the same vendor accepts, e.g. an Anthropic-style endpoint. */
export type PresetAlternate = {
  protocol: CustomProvider['protocol'];
  base_url: string;
  cleanup_models: string[];
};

export type CustomProviderPreset = {
  id: string;
  name: string;
  group: PresetGroup;
  protocol: CustomProvider['protocol'];
  base_url: string;
  requires_key: boolean;
  supports_transcription: boolean;
  supports_cleanup: boolean;
  transcription_models: string[];
  cleanup_models: string[];
  /** Brand color for the monogram tile. */
  color: string;
  /** Overrides the generated initials. */
  mark?: string;
  /** Placeholder for the API key field. */
  key_hint?: string;
  /** One line shown under the picker tile and in the editor. */
  note: string;
  /** Where the user finds their key or setup instructions. */
  docs?: string;
  /** Other API formats this vendor supports. The preset's own protocol is the recommended one. */
  alt?: PresetAlternate;
};

/** Cards shown first, in order. They are omitted from their home group to avoid repeats. */
export const POPULAR_PRESET_IDS = ['ollama', 'lm-studio', 'mistral', 'deepseek', 'together', 'fireworks', 'cerebras', 'anthropic', 'cloudflare'];

export const PRESET_GROUPS: { id: PresetGroup; label: string; hint: string }[] = [
  { id: 'cloud', label: 'Providers', hint: 'Hosted model APIs' },
  { id: 'gateway', label: 'Gateways & developer platforms', hint: 'One key, many models' },
  { id: 'local', label: 'Local & self-hosted', hint: 'Nothing leaves your network' },
  { id: 'advanced', label: 'Advanced', hint: 'Bring any compatible endpoint' },
];

const cloud = (p: Omit<CustomProviderPreset, 'group' | 'protocol' | 'requires_key' | 'supports_cleanup' | 'supports_transcription' | 'transcription_models'> & Partial<CustomProviderPreset>): CustomProviderPreset => ({
  group: 'cloud', protocol: 'openai', requires_key: true, supports_cleanup: true,
  supports_transcription: !!p.transcription_models?.length, transcription_models: [], ...p,
});
const local = (p: Omit<CustomProviderPreset, 'group' | 'protocol' | 'requires_key' | 'supports_cleanup' | 'supports_transcription' | 'transcription_models'> & Partial<CustomProviderPreset>): CustomProviderPreset => ({
  group: 'local', protocol: 'openai', requires_key: false, supports_cleanup: true,
  supports_transcription: !!p.transcription_models?.length, transcription_models: [], ...p,
});
const anthropic = (p: Omit<CustomProviderPreset, 'group' | 'protocol' | 'requires_key' | 'supports_cleanup' | 'supports_transcription' | 'transcription_models'> & Partial<CustomProviderPreset>): CustomProviderPreset => ({
  group: 'cloud', protocol: 'anthropic', requires_key: true, supports_cleanup: true,
  supports_transcription: false, transcription_models: [], ...p,
});

export const CUSTOM_PROVIDER_PRESETS: CustomProviderPreset[] = [
  cloud({ id: 'mistral', name: 'Mistral', color: '#FA520F', base_url: 'https://api.mistral.ai/v1', cleanup_models: ['mistral-small-latest', 'mistral-large-latest'], transcription_models: ['voxtral-mini-latest'], key_hint: 'Paste your Mistral key', note: 'Voxtral transcription and Mistral chat models.', docs: 'https://console.mistral.ai/api-keys' }),
  cloud({ id: 'deepseek', name: 'DeepSeek', color: '#4D6BFE', base_url: 'https://api.deepseek.com/v1', cleanup_models: ['deepseek-chat'], alt: { protocol: 'anthropic', base_url: 'https://api.deepseek.com/anthropic', cleanup_models: ['deepseek-chat'] }, key_hint: 'sk-…', note: 'Low-cost cleanup models.', docs: 'https://platform.deepseek.com/api_keys' }),
  cloud({ id: 'together', name: 'Together AI', color: '#0F6FFF', base_url: 'https://api.together.xyz/v1', cleanup_models: ['meta-llama/Llama-3.3-70B-Instruct-Turbo'], transcription_models: ['openai/whisper-large-v3'], note: 'Open models plus hosted Whisper.', docs: 'https://api.together.ai/settings/api-keys' }),
  cloud({ id: 'fireworks', name: 'Fireworks AI', color: '#6720FF', base_url: 'https://api.fireworks.ai/inference/v1', cleanup_models: ['accounts/fireworks/models/llama-v3p3-70b-instruct'], note: 'Fast open-model inference. Transcription uses a separate audio host.', docs: 'https://fireworks.ai/account/api-keys' }),
  cloud({ id: 'fireworks-audio', name: 'Fireworks Whisper', color: '#6720FF', base_url: 'https://audio-prod.api.fireworks.ai/v1', cleanup_models: [], supports_cleanup: false, transcription_models: ['whisper-v3-turbo', 'whisper-v3'], note: 'Fireworks speech-to-text on its dedicated audio host.', docs: 'https://fireworks.ai/account/api-keys' }),
  cloud({ id: 'cerebras', name: 'Cerebras', color: '#F15A29', base_url: 'https://api.cerebras.ai/v1', cleanup_models: ['llama-3.3-70b'], note: 'Very fast cleanup. Check their current model list.', docs: 'https://cloud.cerebras.ai/platform' }),
  cloud({ id: 'sambanova', name: 'SambaNova', color: '#EE7624', base_url: 'https://api.sambanova.ai/v1', cleanup_models: ['Meta-Llama-3.3-70B-Instruct'], note: 'Fast open-model cleanup.', docs: 'https://cloud.sambanova.ai/apis' }),
  cloud({ id: 'deepinfra', name: 'DeepInfra', color: '#2E6BFF', base_url: 'https://api.deepinfra.com/v1/openai', cleanup_models: ['meta-llama/Llama-3.3-70B-Instruct-Turbo'], transcription_models: ['openai/whisper-large-v3-turbo'], note: 'Pay-per-use open models and Whisper.', docs: 'https://deepinfra.com/dash/api_keys' }),
  cloud({ id: 'hyperbolic', name: 'Hyperbolic', color: '#6D5BFF', base_url: 'https://api.hyperbolic.xyz/v1', cleanup_models: ['meta-llama/Llama-3.3-70B-Instruct'], note: 'Open models on shared GPUs.', docs: 'https://app.hyperbolic.xyz/settings' }),
  cloud({ id: 'nebius', name: 'Nebius AI Studio', color: '#C6F244', base_url: 'https://api.studio.nebius.com/v1', cleanup_models: ['meta-llama/Llama-3.3-70B-Instruct'], note: 'European-hosted open models.', docs: 'https://studio.nebius.com/settings/api-keys' }),
  cloud({ id: 'novita', name: 'Novita AI', color: '#23D57C', base_url: 'https://api.novita.ai/v3/openai', cleanup_models: ['meta-llama/llama-3.3-70b-instruct'], note: 'Open models with simple billing.', docs: 'https://novita.ai/settings/key-management' }),
  cloud({ id: 'perplexity', name: 'Perplexity', color: '#20808D', base_url: 'https://api.perplexity.ai', cleanup_models: ['sonar'], note: 'Sonar models search the web, which cleanup rarely needs.', docs: 'https://www.perplexity.ai/settings/api' }),
  cloud({ id: 'moonshot', name: 'Moonshot (Kimi)', color: '#16191E', base_url: 'https://api.moonshot.ai/v1', cleanup_models: ['kimi-k2-0905-preview'], alt: { protocol: 'anthropic', base_url: 'https://api.moonshot.ai/anthropic', cleanup_models: ['kimi-k2-0905-preview'] }, note: 'Kimi models from Moonshot AI.', docs: 'https://platform.moonshot.ai/console/api-keys' }),
  cloud({ id: 'zai', name: 'Z.ai (GLM)', color: '#2D5BFF', base_url: 'https://api.z.ai/api/paas/v4', cleanup_models: ['glm-4.5-air'], alt: { protocol: 'anthropic', base_url: 'https://api.z.ai/api/anthropic', cleanup_models: ['glm-4.5-air'] }, note: 'GLM models from Zhipu.', docs: 'https://z.ai/manage-apikey/apikey-list' }),
  cloud({ id: 'qwen', name: 'Alibaba Qwen', color: '#615CED', base_url: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1', cleanup_models: ['qwen-flash'], note: 'International DashScope endpoint.', docs: 'https://bailian.console.alibabacloud.com/' }),
  cloud({ id: 'github-models', group: 'gateway', name: 'GitHub Models', color: '#24292F', base_url: 'https://models.github.ai/inference', cleanup_models: ['openai/gpt-4.1-mini'], key_hint: 'GitHub token with models access', note: 'Use a GitHub personal access token.', docs: 'https://github.com/marketplace/models' }),
  cloud({ id: 'vercel', group: 'gateway', name: 'Vercel AI Gateway', color: '#111111', base_url: 'https://ai-gateway.vercel.sh/v1', cleanup_models: ['openai/gpt-4o-mini', 'anthropic/claude-haiku-4.5'], note: 'One key for many vendors, prefixed as vendor/model.', docs: 'https://vercel.com/dashboard' }),
  cloud({ id: 'cloudflare', group: 'gateway', name: 'Cloudflare Workers AI', color: '#F6821F', base_url: 'https://api.cloudflare.com/client/v4/accounts/YOUR_ACCOUNT_ID/ai/v1', cleanup_models: ['@cf/meta/llama-3.3-70b-instruct-fp8-fast'], transcription_models: ['@cf/openai/whisper-large-v3-turbo'], note: 'Replace YOUR_ACCOUNT_ID in the base URL with your Cloudflare account ID.', docs: 'https://dash.cloudflare.com/profile/api-tokens' }),
  cloud({ id: 'openai-compatible', group: 'advanced', name: 'Other OpenAI-compatible', mark: '+', color: '#10A37F', base_url: '', cleanup_models: [], supports_cleanup: true, supports_transcription: false, note: 'Any service that implements /chat/completions.' }),

  anthropic({ id: 'anthropic', name: 'Anthropic', color: '#D97757', base_url: 'https://api.anthropic.com/v1', cleanup_models: ['claude-haiku-4-5', 'claude-sonnet-4-5'], key_hint: 'sk-ant-…', note: 'Claude models through the Messages API.', docs: 'https://console.anthropic.com/settings/keys' }),
  anthropic({ id: 'minimax', name: 'MiniMax', color: '#F23F5D', base_url: 'https://api.minimax.io/anthropic', cleanup_models: ['MiniMax-M2'], note: 'MiniMax exposes an Anthropic-style endpoint.', docs: 'https://platform.minimax.io/' }),
  anthropic({ id: 'anthropic-compatible', group: 'advanced', name: 'Other Anthropic-compatible', mark: '+', color: '#D97757', base_url: '', cleanup_models: [], note: 'Any service that implements /messages.' }),

  local({ id: 'ollama', name: 'Ollama', color: '#262626', base_url: 'http://localhost:11434/v1', cleanup_models: ['llama3.2', 'qwen3:4b'], note: 'Run `ollama pull llama3.2` first. No key needed.', docs: 'https://ollama.com/library' }),
  local({ id: 'lm-studio', name: 'LM Studio', color: '#5B4BFF', base_url: 'http://localhost:1234/v1', cleanup_models: [], note: 'Start the local server in LM Studio and enter the loaded model ID.', docs: 'https://lmstudio.ai/docs/app/api' }),
  local({ id: 'llama-cpp', name: 'llama.cpp server', color: '#7B5E3B', base_url: 'http://localhost:8080/v1', cleanup_models: ['default'], note: 'Run `llama-server -m model.gguf`.', docs: 'https://github.com/ggml-org/llama.cpp/tree/master/tools/server' }),
  local({ id: 'vllm', name: 'vLLM', color: '#30A2FF', base_url: 'http://localhost:8000/v1', cleanup_models: [], note: 'Use the model name you passed to `vllm serve`.', docs: 'https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html' }),
  local({ id: 'speaches', name: 'Speaches (local Whisper)', color: '#0E9F6E', base_url: 'http://localhost:8000/v1', cleanup_models: [], supports_cleanup: false, transcription_models: ['Systran/faster-whisper-small'], note: 'Self-hosted speech-to-text with an OpenAI-style API.', docs: 'https://speaches.ai/' }),
  local({ id: 'localai', name: 'LocalAI', color: '#3B82F6', base_url: 'http://localhost:8080/v1', cleanup_models: [], transcription_models: ['whisper-1'], supports_transcription: true, note: 'Self-hosted chat and Whisper.', docs: 'https://localai.io/' }),
  local({ id: 'jan', name: 'Jan', color: '#F59E0B', base_url: 'http://localhost:1337/v1', cleanup_models: [], note: 'Turn on the local API server in Jan settings.', docs: 'https://jan.ai/docs/desktop/api-server' }),

  { id: 'blank', name: 'Start from scratch', mark: '+', group: 'advanced', protocol: 'openai', base_url: '', requires_key: true, supports_transcription: true, supports_cleanup: true, transcription_models: [], cleanup_models: [], color: '#6B7280', note: 'Enter every field yourself.' },
];

export const presetById = (id: string) => CUSTOM_PROVIDER_PRESETS.find(p => p.id === id);

/** Host portion for display, without scheme, path, or port noise. */
export function displayHost(url: string): string {
  try { return new URL(url).host; } catch { return url; }
}

export function monogram(name: string): string {
  const words = name.replace(/[^\p{L}\p{N} ]/gu, '').trim().split(/\s+/).filter(Boolean);
  if (!words.length) return '?';
  if (words.length > 1) return (words[0][0] + words[1][0]).toUpperCase();
  const inner = words[0].slice(1).match(/[A-Z]/);
  return (inner ? words[0][0] + inner[0] : words[0].slice(0, 2)).toUpperCase();
}

/** Closest preset by hostname, so existing providers keep a recognizable tile. */
export function presetForUrl(url: string, protocol: string): CustomProviderPreset | undefined {
  const host = displayHost(url);
  if (!host) return undefined;
  return CUSTOM_PROVIDER_PRESETS.find(p => p.id !== 'cloudflare' && [{ protocol: p.protocol, base_url: p.base_url }, ...(p.alt ? [p.alt] : [])].some(f => f.base_url && f.protocol === protocol && displayHost(f.base_url) === host));
}
