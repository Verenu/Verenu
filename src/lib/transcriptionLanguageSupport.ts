import type { ProviderId } from './settings';
import type { TranscriptionLanguageCode } from './transcriptionLanguages';

export type LanguageSupportScope = 'all' | TranscriptionLanguageCode[];

const LOCAL_MODEL_LANGUAGES: Record<string, LanguageSupportScope> = {
  'fluid-parakeet-110m': ['en'],
  'fluid-parakeet-ja': ['ja'],
  'fluid-parakeet-ultra': ['bg', 'hr', 'cs', 'da', 'nl', 'en', 'et', 'fi', 'fr', 'de', 'el', 'hu', 'it', 'lv', 'lt', 'pl', 'pt', 'ro', 'sk', 'sl', 'es', 'sv', 'ru', 'uk'],
  'whisper-small': 'all',
  'whisper-large-v3-turbo': 'all',
  'parakeet-v3': [
    'bg', 'hr', 'cs', 'da', 'nl', 'en', 'et', 'fi', 'fr', 'de', 'el', 'hu', 'it', 'lv', 'lt',
    'pl', 'pt', 'ro', 'sk', 'sl', 'es', 'sv', 'ru', 'uk',
  ],
  'parakeet-v2': ['en'],
  'moonshine-base': ['en'],
  'moonshine-tiny': ['en'],
  'moonshine-small': ['en'],
  'moonshine-medium': ['en'],
  'sense-voice': ['zh', 'en', 'ja', 'ko'],
  'gigaam-v3': ['ru'],
  'canary-180m-flash': ['en', 'de', 'es', 'fr'],
  'canary-1b-v2': [
    'bg', 'hr', 'cs', 'da', 'nl', 'en', 'et', 'fi', 'fr', 'de', 'el', 'hu', 'it', 'lv', 'lt',
    'pl', 'pt', 'ro', 'sk', 'sl', 'es', 'sv', 'ru', 'uk',
  ],
  cohere: ['en', 'fr', 'de', 'it', 'es', 'pt', 'el', 'nl', 'pl', 'zh', 'ja', 'ko', 'vi', 'ar'],
};

const CLOUD_PROVIDER_LANGUAGES: Record<
  'groq' | 'openai' | 'google' | 'openrouter' | 'xai',
  LanguageSupportScope
> = {
  groq: 'all',
  openai: 'all',
  google: 'all',
  openrouter: 'all',
  xai: 'all',
};

/**
 * AssemblyAI, unlike the other cloud providers, has a real per-model split:
 * Universal 3.5 Pro natively covers only 6 languages, while Universal-2 covers
 * the same 99+ languages our full dropdown is drawn from.
 */
const ASSEMBLYAI_MODEL_LANGUAGES: Record<string, LanguageSupportScope> = {
  'universal-3-5-pro': ['en', 'es', 'de', 'fr', 'pt', 'it'],
  'universal-2': 'all',
};

export function getLanguageSupport(provider: ProviderId, modelId: string): LanguageSupportScope {
  if (provider === 'local') return LOCAL_MODEL_LANGUAGES[modelId] ?? 'all';
  if (provider === 'assemblyai') return ASSEMBLYAI_MODEL_LANGUAGES[modelId] ?? 'all';
  if (provider.startsWith('custom:')) return 'all';
  return CLOUD_PROVIDER_LANGUAGES[provider as keyof typeof CLOUD_PROVIDER_LANGUAGES] ?? 'all';
}
