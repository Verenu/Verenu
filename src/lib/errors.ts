import { isAndroid, isMac } from './platform';
import type { SettingsSectionId } from './settingsSections';

/**
 * Shared frontend error classification.
 *
 * Backend errors arrive over IPC as strings (command `Err` payloads or the
 * `verenu:error` event). Some of those strings are already user-crafted, some
 * carry structured markers (AUTH_401|..., QUOTA_EXCEEDED:...), and some could
 * still embed provider response bodies. This module is the single place that
 * turns any of those into a stable `ErrorKind` plus a safe, human message, so
 * components stop substring-matching backend strings in their own scripts.
 */

export type ErrorKind =
  | 'unknown'
  | 'already-recording'
  | 'mic-permission'
  | 'accessibility-permission'
  | 'auth-401'
  | 'quota'
  | 'too-short'
  | 'too-quiet'
  | 'nothing-transcribed'
  | 'no-speech'
  | 'local-model-missing'
  | 'no-backend'
  | 'invalid-backup'
  | 'unsupported-backup'
  | 'duplicate'
  | 'storage-full'
  | 'mic-unavailable'
  | 'mic-busy'
  | 'audio-device'
  | 'network'
  | 'timeout'
  | 'provider-unavailable'
  | 'provider-response'
  | 'missing-key'
  | 'model-unavailable'
  | 'local-runtime'
  | 'model-download'
  | 'database'
  | 'credential-store'
  | 'file-permission'
  | 'pairing-code'
  | 'sync-connection'
  | 'sync-protocol'
  | 'retry-expired'
  | 'recording-interrupted'
  | 'recording-too-large'
  | 'file-missing'
  | 'clipboard'
  | 'hotkey';

interface ClassifiedError {
  kind: ErrorKind;
  message: string;
}

/** Order matters: checked top to bottom, first match wins. */
const KIND_HINTS: ReadonlyArray<readonly [ErrorKind, readonly string[]]> = [
  ['already-recording', ['already recording']],
  [
    'mic-permission',
    ['microphone access is blocked', 'microphone access denied', 'microphone permission'],
  ],
  ['accessibility-permission', ['accessibility permission']],
  ['auth-401', ['auth_401', 'invalid or revoked', 'rejected this key', 'rejected authentication']],
  ['quota', ['quota exceeded', 'quota reached', 'request limit reached', 'request limit was reached']],
  // "Recording was too quiet — nothing was transcribed" must classify as
  // nothing-transcribed (the original mic-button wording); keep it before
  // the too-quiet hint.
  ['no-speech', ['no speech detected']],
  ['nothing-transcribed', ['nothing was transcribed', 'nothing transcribed']],
  ['too-short', ['too short']],
  ['too-quiet', ['too quiet']],
  ['local-model-missing', ['download the selected local model', 'download the selected local cleanup model', 'local model is not loaded', 'local cleanup model is not loaded']],
  ['no-backend', ['no configured transcription backend', 'no model in chain produced output']],
  ['invalid-backup', ['invalid backup']],
  ['unsupported-backup', ['unsupported backup']],
  ['duplicate', ['unique constraint']],
  [
    'storage-full',
    [
      'storage_full',
      'storage is full',
      'disk is full',
      'disk full',
      'database or disk is full',
      'no space left on device',
      'not enough space',
      'not enough disk space',
      'insufficient disk space',
      'os error 112',
      'error 112',
    ],
  ],
  ['mic-unavailable', ['no microphone detected', 'no input device available', 'no audio input device']],
  ['mic-busy', ['device or resource busy', 'devicebusy', 'microphone is in use']],
  ['audio-device', ['snd_pcm_', 'alsa::', 'unsupported sample format', 'audio device reported zero channels', 'the requested device is no longer available']],
  ['missing-key', ['no api key saved', 'no saved api key', 'api key is not configured']],
  ['retry-expired', ['retry window expired', 'no retry available', 'nothing to resume']],
  ['recording-interrupted', ['recording thread', 'failed to stop recording', 'no active recording', 'no audio captured']],
  ['pairing-code', ["that code didn't match", 'pairing code must', 'incorrect pairing code']],
  ['sync-protocol', ['peer identity', 'pairing handshake', 'unexpected pairing message', 'unexpected pairing response', 'unexpected message during pull', 'expected hello', 'expected meta', 'expected sync completion', 'peer ended sync', 'invalid dictionary payload', 'invalid snippet payload', 'invalid context payload', 'bad nonce length', 'bad key length']],
  ['sync-connection', ['no reachable address', 'could not bind sync listener', 'mdns', 'sync session with', "didn't complete the pairing in time"]],
  ['local-runtime', ['local cleanup runtime unavailable', 'local cleanup runtime not installed', 'local cleanup runtime timed out', 'local cleanup runtime exited', 'runtime is not installed', 'llama-server', 'missing its primary gguf file']],
  ['model-download', ['checksum mismatch', 'sha256 mismatch', 'hash mismatch', 'failed checksum verification', 'another local model download', 'another local cleanup model download']],
  ['model-unavailable', ['unknown local model', 'unknown local cleanup model', 'invalid model identifier']],
  ['database', ['database is locked', 'database lock', 'database disk image is malformed', 'no such table', 'sqlite']],
  ['credential-store', ['keychain', 'secret service', 'credential store', 'credential manager', 'secure credential backend', 'keystore']],
  ['clipboard', ['wayland clipboard unavailable', 'could not write wayland clipboard', 'could not read wayland clipboard', 'clipboard access denied']],
  ['hotkey', ['unrecognized key code', 'hotkey may already be in use', 'failed to save hotkey']],
  ['file-permission', ['permission denied', 'access is denied', 'os error 5', 'os error 13']],
  ['file-missing', ['no such file or directory', 'os error 2', 'system cannot find the file', 'downloaded update installer is missing']],
  ['timeout', ['timed out', 'timeout']],
  ['network', ['error sending request', 'dns error', 'failed to resolve host', 'failed to resolve address', 'connection refused', 'network is unreachable', 'could not resolve host', "couldn't reach", 'could not reach', 'could not connect to', 'connection reset', 'tls handshake']],
  ['provider-response', ['no choices in', 'no candidates', 'no transcript', 'response parse error', 'finish_reason', 'gemini blocked']],
];

/** Copy for kinds whose wording is fixed and shared across surfaces. */
const KIND_MESSAGES: Partial<Record<ErrorKind, string>> = {
  'already-recording': 'Already recording. Finish or cancel the current recording before starting another.',
  'mic-permission': isAndroid
    ? 'Microphone access is blocked. Allow microphone access for Verenu in Android Settings, then try again.'
    : 'Microphone access is blocked. Allow microphone access for Verenu in your system privacy settings, then try again.',
  'accessibility-permission': isAndroid
    ? 'Verenu cannot insert text. Enable the Verenu accessibility service in Android Settings, then try again.'
    : 'Verenu cannot insert text. Enable Verenu in System Settings > Privacy & Security > Accessibility, then try again.',
  'too-short': 'The recording was too short to transcribe. Record a longer phrase and stop after you finish speaking.',
  'too-quiet': 'The microphone captured very quiet audio. Check that it is unmuted, move closer, or raise its gain in Settings > Audio.',
  'nothing-transcribed': 'No speech was transcribed. Check your microphone and selected language, then try again.',
  'no-speech': 'No speech was detected in this recording. Check that your microphone is unmuted, move closer, then try again.',
  'local-model-missing': 'The selected local model is not ready. Download it in Settings > Models, or choose a cloud model.',
  'no-backend': 'No transcription model is available. Choose a model in Settings > Models and add its API key if it uses a cloud provider.',
  'invalid-backup': "That file isn't a valid Verenu backup.",
  'unsupported-backup':
    "This backup was made by a newer Verenu version and can't be read. Update Verenu to import it.",
  duplicate: 'An entry with that value already exists. Edit the existing entry or use a different value.',
  'storage-full': 'There is not enough free storage to complete this action. Free up space, then try again.',
  'mic-unavailable': 'No microphone detected. Reconnect your mic or choose another input in Settings > General.',
  'mic-busy': 'The audio device is busy. Stop other apps using it, then try recording again.',
  'audio-device': 'The audio system could not open the microphone. Reconnect it or choose another input in Settings > General.',
  'missing-key': 'No API key is saved for the selected provider. Add it in Settings > Providers, or choose a local model.',
  'retry-expired': 'This recording is no longer available to retry or resume. Start a new dictation.',
  'recording-interrupted': 'The recording could not be completed. Check your microphone connection, then start a new dictation.',
  'recording-too-large': 'The provider could not accept a recording this large. Try a shorter dictation or choose another transcription provider.',
  'file-missing': 'The required file or folder could not be found. Select an existing file, or download it again if it is a model or update.',
  'pairing-code': 'The pairing code did not match. Enter the six-digit code shown on the other device, or start pairing again.',
  'sync-connection': 'Verenu could not reach the other device. Open Verenu on both devices, connect them to the same local network, and try again.',
  'sync-protocol': 'The devices could not complete a secure sync. Update Verenu on both devices and try again. If it persists, remove and pair the device again.',
  'local-runtime': 'The local cleanup engine could not start. Check its installation in Settings > Models, or choose a cloud cleanup model.',
  'model-download': 'The downloaded model failed its integrity check. Retry the download in Settings > Models.',
  'model-unavailable': 'The selected model is not available. Choose another model in Settings > Models.',
  database: 'Verenu could not access its local data. Try again. If it keeps happening, restart Verenu.',
  'credential-store': 'Verenu could not access secure key storage. Unlock your system keyring or approve the credential access prompt, then try again.',
  clipboard: 'Verenu could not access the clipboard. Check your system clipboard permissions, then try again.',
  hotkey: 'Verenu could not register that shortcut. Choose a different key combination in Settings > General.',
  'file-permission': 'Verenu does not have permission to access the file or folder. Check its permissions or choose another location, then try again.',
  timeout: 'The request took too long to finish. Check your connection, then try again.',
  network: 'Verenu could not connect to the service. Check your internet connection and any VPN or firewall, then try again.',
  'provider-response': 'The provider did not return a usable result. Try again, or choose another model in Settings > Models.',
  'provider-unavailable': 'The provider is temporarily unavailable. Wait a moment, then try again or choose another provider.',
};

/** Extracts a displayable string from any IPC/JS error shape. */
export function extractIpcErrorMessage(err: unknown): string {
  if (typeof err === 'object' && err !== null) {
    if ('message' in err) {
      const message = (err as { message?: unknown }).message;
      if (typeof message === 'string' && message.trim()) {
        return message.trim();
      }
    }
    if ('error' in err) {
      const error = (err as { error?: unknown }).error;
      if (typeof error === 'string' && error.trim()) {
        return error.trim();
      }
    }
  }
  if (err instanceof Error && err.message?.trim()) {
    return err.message.trim();
  }
  const raw = String(err ?? '').trim();
  if (!raw || raw === '[object Object]') {
    return 'Verenu could not complete this action. Try again. If it keeps happening, restart Verenu.';
  }
  return raw;
}

/**
 * Defense-in-depth for strings that bypass the Rust-side sanitizer (e.g. a
 * future command that returns a raw provider context string). Strips response
 * bodies and internal request ids. Keep complete recovery instructions visible.
 */
function stripInternalMarkers(message: string): string {
  const cleaned = message
    .replace(/(?:—|-) recent runtime log:[\s\S]*$/g, '')
    .replace(/body_preview=[\s\S]*$/g, '')
    .replace(/request_id=[^\s|]+/g, '')
    .replace(/[ \t]{2,}/g, ' ')
    .trim();
  return cleaned.length > 600 ? `${cleaned.slice(0, 600).replace(/\s+\S*$/, '')}…` : cleaned;
}

function messageForKind(kind: ErrorKind, raw: string): string {
  const lower = raw.toLowerCase();
  switch (kind) {
    case 'auth-401': {
      if (raw.includes('AUTH_401')) {
        const provider = raw.match(/provider=(Groq|OpenAI|Google|Gemini|AssemblyAI|OpenRouter|xAI)\b/i)?.[1] ?? 'The provider';
        if (lower.includes('category=invalid_or_revoked_key')) {
          return `${provider} API key looks invalid or revoked. Replace it in Settings > Providers.`;
        }
        return lower.includes('category=scope_or_account_restriction')
          ? `${provider} rejected this key for account or model access. Check the key's permissions and your provider account's access to the selected model.`
          : `${provider} rejected authentication. Replace the key in Settings > Providers and check your provider account's access.`;
      }
      return stripInternalMarkers(raw);
    }
    case 'quota': {
      // "QUOTA_EXCEEDED: Groq quota reached" or the already-friendly message.
      const provider = raw.match(/\b(Groq|OpenAI|Google|Gemini|AssemblyAI|OpenRouter|xAI)\b/i)?.[0];
      return `${provider ?? 'Your provider'} request limit reached. Wait for the limit to reset, check your provider plan, or choose another provider.`;
    }
    case 'model-download':
      return lower.includes('another local')
        ? 'Another model download is already running. Wait for it to finish or cancel it before starting this download.'
        : KIND_MESSAGES['model-download']!;
    case 'nothing-transcribed':
      return lower.includes('too quiet') ? KIND_MESSAGES['too-quiet']! : KIND_MESSAGES['nothing-transcribed']!;
    case 'database':
      return lower.includes('malformed')
        ? 'Verenu could not read its local database because it appears damaged. Restart Verenu. If it persists, restore a Verenu backup in Settings > Privacy.'
        : KIND_MESSAGES.database!;
    case 'credential-store':
      if (lower.includes('no secure credential backend')) {
        return 'This build cannot store API keys securely. Install a supported Verenu release, then save your key again.';
      }
      if (lower.includes('entitlement') || lower.includes('-34018')) {
        return 'macOS rejected secure key storage for this Verenu build. Install the signed Verenu release, then save your key again.';
      }
      if (lower.includes('secret service')) {
        return 'Verenu could not access your system keyring. Unlock GNOME Keyring or KWallet, then try again.';
      }
      if (lower.includes('verify') || lower.includes('verification')) {
        return 'Verenu could not verify that your API key was saved. Unlock secure key storage and save the key again before dictating.';
      }
      return KIND_MESSAGES['credential-store']!;
    case 'recording-interrupted':
      return lower.includes('no active recording')
        ? 'There is no recording to stop. Start a new dictation first.'
        : KIND_MESSAGES['recording-interrupted']!;
    case 'file-missing':
      return lower.includes('downloaded update installer')
        ? 'The downloaded update installer is missing. Check for updates again in Settings > About to download a fresh installer.'
        : KIND_MESSAGES['file-missing']!;
    default: {
      return KIND_MESSAGES[kind] ?? stripInternalMarkers(raw);
    }
  }
}

export function classifyIpcError(err: unknown): ClassifiedError {
  const raw = extractIpcErrorMessage(err);
  // Never classify the provider body as the cause of a local app failure.
  const safe = stripInternalMarkers(raw);
  const lower = safe.toLowerCase();
  const status = safe.match(/(?:HTTP\s+|status[=:]?\s+|status[=:])(4\d\d|5\d\d)\b/i)?.[1];
  if (status && !lower.includes('auth_401')) {
    const code = Number(status);
    if (code === 401 || code === 403) return { kind: 'auth-401', message: 'The provider rejected access. Check your API key and account access in Settings > Providers.' };
    if (code === 429) return { kind: 'quota', message: messageForKind('quota', safe) };
    if (code === 408 || code === 504) return { kind: 'timeout', message: KIND_MESSAGES.timeout! };
    if (code >= 500) return { kind: 'provider-unavailable', message: KIND_MESSAGES['provider-unavailable']! };
    if (code === 404) return { kind: 'model-unavailable', message: 'The requested model or download was not found. Refresh the model list in Settings > Models and choose an available model.' };
    if (code === 413) return { kind: 'recording-too-large', message: KIND_MESSAGES['recording-too-large']! };
    return { kind: 'provider-response', message: `The provider rejected the request (HTTP ${code}). Check the selected model and language in Settings > Models, then try again.` };
  }
  for (const [kind, hints] of KIND_HINTS) {
    if (hints.some((hint) => lower.includes(hint))) {
      return { kind, message: messageForKind(kind, safe) };
    }
  }
  if (/lock.*poison|panicked|task failed|channel.*closed|invalid args|command.*not found|\[object Object\]/i.test(safe)) {
    return { kind: 'unknown', message: 'Verenu could not complete this action. Try again. If it keeps happening, restart Verenu.' };
  }
  if (raw.includes('body_preview=') || raw.includes('request_id=')) {
    return { kind: 'provider-response', message: KIND_MESSAGES['provider-response']! };
  }
  return { kind: 'unknown', message: safe };
}

/** Add the failed action when a native error only describes its cause. */
export function formatIpcError(err: unknown, action?: string): string {
  const { message } = classifyIpcError(err);
  return action && !message.toLowerCase().startsWith(action.replace(/[.!?]+$/, '').toLowerCase())
    ? `${action.replace(/[.!?]+$/, '')}. ${message}` : message;
}

/** LAN sync uses the local network, so do not send people to internet settings. */
export function formatSyncError(err: unknown, action?: string): string {
  const raw = stripInternalMarkers(extractIpcErrorMessage(err));
  const classified = classifyIpcError(err);
  let message = classified.message;
  if (classified.kind === 'network' || classified.kind === 'timeout') {
    message = KIND_MESSAGES['sync-connection']!;
  }
  if (/tls|certificate|identity encryption|pairing handshake/i.test(raw) && !/timed out|timeout/i.test(raw)) {
    message = KIND_MESSAGES['sync-protocol']!;
  }
  const lower = raw.toLowerCase();
  if (lower.includes('already running') || lower.includes('pairing is already in progress')) {
    message = 'A sync or pairing session is already running. Wait for it to finish, or cancel pairing before starting again.';
  } else if (lower.includes('no incoming pairing') || lower.includes('pairing was cancelled')) {
    message = 'This pairing request is no longer active. Start pairing again from Settings > Sync on the other device.';
  } else if (lower.includes('not paired') || lower.includes('no paired devices')) {
    message = 'There is no paired connection to this device. Pair it in Settings > Sync before syncing.';
  } else if (lower.includes('not visible') || lower.includes('no longer visible') || lower.includes('no connection candidates')) {
    message = KIND_MESSAGES['sync-connection']!;
  } else if (lower.includes('sync unavailable')) {
    message = 'Sync could not start on this device. Restart Verenu and check that your system keyring is unlocked.';
  } else if (lower.includes('device name cannot be empty')) {
    message = 'Enter a device name before saving it.';
  }
  const prefix = action?.replace(/[.!?]+$/, '');
  return prefix && !message.toLowerCase().startsWith(prefix.toLowerCase()) ? `${prefix}. ${message}` : message;
}

/**
 * Which settings section, if any, is the useful next step for this error.
 * `null` means there is nothing actionable to jump to.
 */
export function settingsSectionForKind(kind: ErrorKind): SettingsSectionId | null {
  switch (kind) {
    case 'auth-401':
    case 'quota':
    case 'missing-key':
      return 'keys';
    case 'no-backend':
    case 'local-model-missing':
    case 'local-runtime':
    case 'model-download':
    case 'model-unavailable':
    case 'provider-response':
      return 'models';
    case 'mic-permission':
    case 'accessibility-permission':
      return isMac ? 'permissions' : 'advanced';
    case 'mic-unavailable':
    case 'mic-busy':
    case 'audio-device':
    case 'too-quiet':
      return kind === 'too-quiet' ? 'advanced' : 'general';
    case 'hotkey':
      return 'general';
    case 'sync-connection':
    case 'sync-protocol':
    case 'pairing-code':
      return 'sync';
    default:
      return null;
  }
}
