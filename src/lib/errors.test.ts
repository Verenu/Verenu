import { describe, expect, it } from 'vitest';
import { classifyIpcError, formatIpcError, formatSyncError, settingsSectionForKind, type ErrorKind } from './errors';

describe('storage-full errors', () => {
  it('recognizes the backend marker', () => {
    expect(classifyIpcError('STORAGE_FULL: simulated settings write failure')).toEqual({
      kind: 'storage-full',
      message: 'There is not enough free storage to complete this action. Free up space, then try again.',
    });
  });

  it('recognizes native full-disk wording', () => {
    expect(classifyIpcError('Failed to write settings: No space left on device').kind).toBe(
      'storage-full',
    );
  });
});

describe('actionable errors', () => {
  it.each<[string, ErrorKind, string]>([
    ['Failed to start recording: No input device available', 'mic-unavailable', 'Settings > General'],
    ["ALSA function 'snd_pcm_hw_params' failed with error 'No such device'", 'audio-device', 'microphone'],
    ['Failed to start recording: Device or resource busy', 'mic-busy', 'Stop other apps'],
    ['Recording too short', 'too-short', 'longer phrase'],
    ['No speech detected', 'no-speech', 'unmuted'],
    ['Audio too quiet - check your mic', 'too-quiet', 'unmuted'],
    ['Nothing transcribed - please try speaking more clearly', 'nothing-transcribed', 'selected language'],
    ['No API key saved for groq', 'missing-key', 'Settings > API Keys'],
    ['No configured transcription backend is available', 'no-backend', 'Settings > Models'],
    ['Download the selected local cleanup model.', 'local-model-missing', 'Download'],
    ['download checksum mismatch', 'model-download', 'integrity check'],
    ['Downloaded model failed checksum verification', 'model-download', 'integrity check'],
    ['Insufficient disk space for model download', 'storage-full', 'Free up space'],
    ['another local model download is already running', 'model-download', 'Wait for it'],
    ['Keychain write failed: -34018', 'credential-store', 'signed Verenu release'],
    ['Secret Service is unavailable or locked', 'credential-store', 'Unlock GNOME'],
    ['Keychain verification failed: item missing', 'credential-store', 'save the key again'],
    ['database is locked', 'database', 'restart Verenu'],
    ['database disk image is malformed', 'database', 'appears damaged'],
    ['Retry window expired', 'retry-expired', 'new dictation'],
    ['No active recording', 'recording-interrupted', 'no recording to stop'],
    ["that code didn't match", 'pairing-code', 'six-digit code'],
    ['TLS handshake with fixture-device timed out', 'timeout', 'try again'],
    ['sync peer identity does not match the paired device', 'sync-protocol', 'pair the device again'],
    ['error sending request for url (https://example.invalid)', 'network', 'internet connection'],
    ['AssemblyAI transcription timed out waiting for a result', 'timeout', 'try again'],
    ['Gemini returned no candidates', 'provider-response', 'another model'],
    ['Could not write Wayland clipboard: unavailable', 'clipboard', 'clipboard'],
    ['Failed to write backup file: Permission denied', 'file-permission', 'permissions'],
  ])('explains %s', (raw, kind, guidance) => {
    const result = classifyIpcError(raw);
    expect(result.kind).toBe(kind);
    expect(result.message).toContain(guidance);
    expect(result.message).not.toContain('example.invalid');
  });

  it.each([401, 403, 404, 408, 413, 429, 500, 502, 503, 504])('explains HTTP %i without displaying response data', (status) => {
    const result = classifyIpcError(`provider status=${status} request_id=test-request body_preview=private response data`);
    expect(result.kind).not.toBe('unknown');
    expect(result.message).not.toMatch(/private response|test-request|body_preview|request_id/);
  });

  it('does not interpret provider body text as a local storage failure', () => {
    const result = classifyIpcError('provider status=503 body_preview=database or disk is full');
    expect(result.kind).toBe('provider-unavailable');
  });

  it('does not display malformed authentication metadata', () => {
    const result = classifyIpcError('AUTH_401|provider=Groq|category=invalid_or_revoked_key|request_id=private-id');
    expect(result.kind).toBe('auth-401');
    expect(result.message).toContain('Settings > API Keys');
    expect(result.message).not.toMatch(/AUTH_401|request_id|private-id/);
  });

  it('keeps a complete authored recovery message longer than 120 characters', () => {
    const message = 'This backup contains a newer context format that this version cannot read. Update Verenu on this device, then select the same backup and import it again.';
    expect(classifyIpcError(message).message).toBe(message);
  });

  it('adds the failed action to the cause', () => {
    expect(formatIpcError(new Error('No space left on device'), 'Could not export your backup')).toMatch(/^Could not export your backup\. There is not enough free storage/);
  });

  it('distinguishes a cloud connection failure from a LAN sync failure', () => {
    expect(classifyIpcError('Verenu could not connect to the provider').message).toContain('internet connection');
    expect(formatSyncError('could not reach fixture-device: connection refused')).toContain('same local network');
    expect(formatSyncError('TLS handshake timed out')).toContain('same local network');
    expect(formatSyncError('TLS handshake failed: certificate rejected')).toContain('pair the device again');
  });

  it.each([null, undefined, {}, new Error('Recording task panicked: internal details')])('gives a recovery step for an unusable error %j', (error) => {
    expect(classifyIpcError(error).message).toContain('restart Verenu');
  });

  it('routes microphone selection and gain errors to their actual settings pages', () => {
    expect(settingsSectionForKind('mic-unavailable')).toBe('general');
    expect(settingsSectionForKind('too-quiet')).toBe('advanced');
    expect(settingsSectionForKind('missing-key')).toBe('keys');
  });
});
