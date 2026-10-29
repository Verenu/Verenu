export type BuiltinProviderId = 'groq' | 'openai' | 'google' | 'assemblyai' | 'openrouter' | 'xai';

export type BuiltinProvider = {
  id: BuiltinProviderId;
  label: string;
  placeholder: string;
  transcription: boolean;
  cleanup: boolean;
  keyUrl: string;
};

export const BUILTIN_PROVIDERS: BuiltinProvider[] = [
  { id: 'groq', label: 'Groq', placeholder: 'gsk_…', transcription: true, cleanup: true, keyUrl: 'https://console.groq.com/keys' },
  { id: 'openai', label: 'OpenAI', placeholder: 'sk-…', transcription: true, cleanup: true, keyUrl: 'https://platform.openai.com/api-keys' },
  { id: 'google', label: 'Gemini', placeholder: 'AIza…', transcription: true, cleanup: true, keyUrl: 'https://aistudio.google.com/apikey' },
  { id: 'assemblyai', label: 'AssemblyAI', placeholder: '32-character key', transcription: true, cleanup: false, keyUrl: 'https://www.assemblyai.com/dashboard/api-keys' },
  { id: 'openrouter', label: 'OpenRouter', placeholder: 'sk-or-…', transcription: true, cleanup: true, keyUrl: 'https://openrouter.ai/keys' },
  { id: 'xai', label: 'xAI', placeholder: 'xai-…', transcription: true, cleanup: true, keyUrl: 'https://console.x.ai' },
];
