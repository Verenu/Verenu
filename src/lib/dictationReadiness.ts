export type ReadinessIssue = {
  task: 'transcription' | 'cleanup';
  message: string;
  section: 'keys' | 'models';
  action: string;
};

export type ReadinessCustomProvider = {
  id: string;
  name: string;
  requires_key: boolean;
  supports_transcription: boolean;
  supports_cleanup: boolean;
};

export type ReadinessInput = {
  transcriptionModel: string;
  cleanupModel: string;
  transcriptionFallbacks?: string[];
  cleanupFallbacks?: string[];
  cleanupEnabled: boolean;
  keys: Record<string, boolean>;
  speechModels: { id: string; is_downloaded: boolean }[];
  cleanupModels: { id: string; is_downloaded: boolean }[];
  cleanupEngineInstalled: boolean;
  customProviders?: ReadinessCustomProvider[];
};

const CLOUD_PROVIDERS = new Set(['groq', 'openai', 'google', 'assemblyai', 'openrouter', 'xai']);

const DEFAULT_MODELS: Record<string, Partial<Record<ReadinessIssue['task'], string>>> = {
  local: { transcription: 'local/parakeet-v3', cleanup: 'local/qwen2.5-3b-instruct' },
  groq: { transcription: 'groq/whisper-large-v3-turbo', cleanup: 'groq/qwen/qwen3.8-27b' },
  openai: { transcription: 'openai/gpt-4o-transcribe', cleanup: 'openai/gpt-4o-mini' },
  google: { transcription: 'google/gemini-3.5-transcribe', cleanup: 'google/gemini-3.5-flash-lite' },
  assemblyai: { transcription: 'assemblyai/universal-3-5-pro', cleanup: 'groq/qwen/qwen3.8-27b' },
  openrouter: { transcription: 'openrouter/openai/whisper-large-v3', cleanup: 'openrouter/openai/gpt-4o-mini' },
  xai: { transcription: 'xai/grok-voice-transcribe-2.0', cleanup: 'xai/grok-4-fast-non-reasoning' },
};

// Resolve old unqualified model settings without querying any provider.
export function readinessModel(
  task: ReadinessIssue['task'],
  selected: string | null,
  legacy: string | null,
  provider: string | null,
): string {
  const configured = selected?.trim() || legacy?.trim();
  if (configured) {
    const slash = configured.indexOf('/');
    const namespace = slash < 0 ? '' : configured.slice(0, slash);
    if (namespace && (CLOUD_PROVIDERS.has(namespace) || namespace === 'local' || namespace.startsWith('custom:'))) return configured;
    return (provider || 'groq') + '/' + configured;
  }
  const source = provider || 'groq';
  return DEFAULT_MODELS[source]?.[task] ?? source + '/';
}

type CandidateResult = { ready: true } | { ready: false; message: string; section: 'keys' | 'models' };

function evaluateModel(task: ReadinessIssue['task'], value: string, input: ReadinessInput): CandidateResult {
  const slash = value.indexOf('/');
  const provider = value.slice(0, slash);
  const id = value.slice(slash + 1).trim();
  const label = task === 'transcription' ? 'Speech recognition' : 'Optional cleanup';
  if (slash <= 0 || !id) {
    return { ready: false, message: label + ' needs a supported model.', section: 'models' };
  }

  if (provider === 'local') {
    const models = task === 'transcription' ? input.speechModels : input.cleanupModels;
    if (!models.some(model => model.id === id && model.is_downloaded)) {
      return { ready: false, message: label + ' model ' + id + ' is not installed.', section: 'models' };
    }
    if (task === 'cleanup' && !input.cleanupEngineInstalled) {
      return { ready: false, message: 'Optional cleanup needs its local engine.', section: 'models' };
    }
    return { ready: true };
  }

  const custom = input.customProviders?.find(item => item.id === provider);
  if (custom) {
    const supported = task === 'transcription' ? custom.supports_transcription : custom.supports_cleanup;
    if (!supported) {
      return { ready: false, message: custom.name + ' does not support ' + (task === 'transcription' ? 'speech recognition' : 'cleanup') + '.', section: 'models' };
    }
    if (custom.requires_key && !input.keys[provider]) {
      return { ready: false, message: label + ' needs an API key for ' + custom.name + '.', section: 'keys' };
    }
    return { ready: true };
  }

  if (!CLOUD_PROVIDERS.has(provider)) {
    return { ready: false, message: label + ' needs a supported model.', section: 'models' };
  }
  if (task === 'cleanup' && !supportsBuiltinCleanupModel(provider, id)) {
    return { ready: false, message: label + ' model ' + id + ' is not supported for cleanup.', section: 'models' };
  }
  if (!input.keys[provider]) {
    return { ready: false, message: label + ' needs an API key for ' + provider + '.', section: 'keys' };
  }
  return { ready: true };
}

function supportsBuiltinCleanupModel(provider: string, rawModel: string): boolean {
  const model = rawModel.trim().toLowerCase();
  if (provider === 'assemblyai') return false;
  if (provider === 'local') return true;
  if (provider === 'google') {
    if (model.includes('gemini-3')) {
      return model.includes('gemini-3.5-flash-lite') || model.includes('gemini-3.5-flash');
    }
    return model.includes('gemini-2.5-flash') && !model.includes('gemini-2.5-pro');
  }

  if (['groq', 'openai', 'openrouter', 'xai'].includes(provider)) {
    if (model.includes('gpt-oss')) return false;
    if (provider === 'groq') {
      return !model.startsWith('qwen/qwen3') || model.startsWith('qwen/qwen3.6-') || model.startsWith('qwen/qwen3.8-');
    }
    if (provider === 'openrouter') {
      const separator = model.lastIndexOf('/');
      const bare = separator < 0 ? model : model.slice(separator + 1);
      const openAiOSeries = /^o\d/.test(bare);
      return !(
        model.endsWith(':thinking') || bare.includes('-thinking') || bare.includes('reasoner') ||
        bare.includes('-r1') || openAiOSeries ||
        (bare.startsWith('gpt-5') && !bare.startsWith('gpt-5.1'))
      );
    }
    if (provider === 'xai') {
      if (model.includes('non-reasoning')) return true;
      return !(
        model.includes('reasoning') || model.startsWith('grok-4') || model.startsWith('grok-3-mini')
      );
    }
    return !model.startsWith('o') && !(model.startsWith('gpt-5') && !model.startsWith('gpt-5.1'));
  }

  return false;
}

function candidatesFor(task: ReadinessIssue['task'], input: ReadinessInput): string[] {
  const primary = task === 'transcription' ? input.transcriptionModel : input.cleanupModel;
  const fallbacks = task === 'transcription' ? input.transcriptionFallbacks : input.cleanupFallbacks;
  return [...new Set([primary, ...(fallbacks ?? [])].map(model => model.trim()).filter(Boolean))];
}

export function dictationReadiness(input: ReadinessInput): ReadinessIssue[] {
  const issues: ReadinessIssue[] = [];
  for (const task of ['transcription', 'cleanup'] as const) {
    if (task === 'cleanup' && !input.cleanupEnabled) continue;
    const candidates = candidatesFor(task, input);
    const results = (candidates.length ? candidates : ['']).map(model => evaluateModel(task, model, input));
    if (results.some(result => result.ready)) continue;

    const label = task === 'transcription' ? 'Speech recognition' : 'Optional cleanup';
    const problems = [...new Set(results.filter((result): result is Extract<CandidateResult, { ready: false }> => !result.ready).map(result => result.message))];
    const section = results.some(result => !result.ready && result.section === 'keys') ? 'keys' : 'models';
    issues.push({
      task,
      message: problems.length === 1 ? problems[0] : label + ' has no ready configured model. ' + problems.join(' '),
      section,
      action: section === 'keys' ? 'Add API key' : 'Choose models',
    });
  }
  return issues;
}

export function hasReadyOfflineSpeech(input: ReadinessInput): boolean {
  return candidatesFor('transcription', input).some(model => {
    const provider = model.slice(0, model.indexOf('/'));
    const offlineCapable = provider === 'local' || input.customProviders?.some(custom => custom.id === provider) === true;
    return offlineCapable && evaluateModel('transcription', model, input).ready;
  });
}

export function hasCloudSpeechCandidate(input: ReadinessInput): boolean {
  return candidatesFor('transcription', input).some(model => {
    const provider = model.slice(0, model.indexOf('/'));
    return CLOUD_PROVIDERS.has(provider);
  });
}
