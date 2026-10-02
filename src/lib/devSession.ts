export type SessionInfo = {
  id: string; branch: string; commit: string; transport: 'rust-live'; platform: string;
  privateHistory: boolean; maxRuns: number; runs: number; eventCursor: number; shareUrl?: string;
  capabilities: { productionPipeline: boolean; browserAudio: boolean; hostMicrophone: boolean; nativeInjection: boolean; globalHotkeys: boolean; credentialWrites: boolean };
};
type Envelope = { id: number; event: string; payload: unknown };
let info: SessionInfo | null = null;
let token = '';
let cursor = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
const handlers = new Map<string, Set<(event: Envelope) => void>>();
const subscriptions = new Map<string, Promise<void>>();

export function isBrowserDevSession(): boolean {
  return import.meta.env.VITE_VERENU_SESSION === '1' && typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window);
}
export function sessionInfo(): SessionInfo | null { return info; }
export function sessionShareLink(): string | null {
  return info?.shareUrl ? `${info.shareUrl}/#session-token=${token}` : null;
}

export async function sessionRequest<T>(route: string, init: RequestInit = {}): Promise<T> {
  if (!isBrowserDevSession()) throw new Error('No browser dev session is configured');
  if (!token) throw new Error('Open the private session access link to connect');
  const headers = new Headers(init.headers);
  headers.set('Authorization', `Bearer ${token}`);
  const timeout = route.startsWith('/audio') || route === '/invoke' ? 190_000 : 10_000;
  const response = await fetch(`/__verenu_dev${route}`, { ...init, signal: init.signal || AbortSignal.timeout(timeout), headers, cache: 'no-store' });
  const value: unknown = await response.json().catch(() => null);
  if (!response.ok) throw new Error(value && typeof value === 'object' && 'error' in value ? String(value.error) : 'Dev backend is unavailable');
  return value as T;
}

export async function initializeDevSession(): Promise<void> {
  if (!isBrowserDevSession()) return;
  const hash = new URLSearchParams(location.hash.slice(1));
  const access = hash.get('session-token');
  if (access) {
    token = access; sessionStorage.setItem('verenu:session-token', token); hash.delete('session-token');
    history.replaceState(null, '', `${location.pathname}${location.search}${hash.size ? `#${hash}` : ''}`);
  } else { token = sessionStorage.getItem('verenu:session-token') || ''; }
  info = await sessionRequest<SessionInfo>('/session');
  if (info.transport !== 'rust-live' || (import.meta.env.VITE_VERENU_SESSION_ID && info.id !== import.meta.env.VITE_VERENU_SESSION_ID)) throw new Error('The browser is connected to the wrong dev backend');
  cursor = info.eventCursor;
  void poll();
  window.addEventListener('pagehide', () => { if (timer) clearTimeout(timer); }, { once: true });
}

export function sessionInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return sessionRequest<T>('/invoke', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ command, args: args || {} }) });
}
export function sessionEmit<T>(event: string, payload?: T): Promise<void> {
  const envelope = { id: 0, event, payload } as Envelope;
  for (const handler of handlers.get(event) || []) handler(envelope);
  return Promise.resolve();
}
export async function sessionListen<T>(event: string, handler: (event: { id: number; event: string; payload: T }) => void): Promise<() => void> {
  let listeners = handlers.get(event);
  if (!listeners) { listeners = new Set(); handlers.set(event, listeners); }
  const callback = handler as (event: Envelope) => void;
  listeners.add(callback);
  let subscription = subscriptions.get(event);
  if (!subscription) {
    subscription = sessionRequest('/listen', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ event }) }).then(() => {});
    subscriptions.set(event, subscription);
  }
  try { await subscription; } catch (error) { listeners.delete(callback); subscriptions.delete(event); throw error; }
  return () => { listeners?.delete(callback); };
}
async function poll(): Promise<void> {
  try {
    const result = await sessionRequest<{ cursor: number; gap: boolean; events: Envelope[] }>(`/events?after=${cursor}`);
    if (result.gap) window.dispatchEvent(new CustomEvent('verenu:dev-connection', { detail: 'Backend events were missed. Repeat this verification.' }));
    for (const event of result.events) for (const handler of handlers.get(event.event) || []) {
      try { handler(event); } catch { window.dispatchEvent(new CustomEvent('verenu:dev-connection', { detail: 'A backend event handler failed. Repeat this verification.' })); }
    }
    cursor = result.cursor;
  } catch { window.dispatchEvent(new CustomEvent('verenu:dev-connection', { detail: 'The Rust backend disconnected. Verification is incomplete.' })); }
  timer = setTimeout(() => { void poll(); }, 250);
}

export function encodePcmWav(samples: Float32Array, sampleRate = 16_000): Uint8Array<ArrayBuffer> {
  if (sampleRate !== 16_000 || samples.length > 120 * sampleRate) throw new Error('Use at most 120 seconds of 16 kHz mono samples');
  const bytes = new Uint8Array(44 + samples.length * 2);
  const view = new DataView(bytes.buffer);
  const write = (offset: number, value: string) => { for (let i = 0; i < value.length; i++) bytes[offset + i] = value.charCodeAt(i); };
  write(0, 'RIFF'); view.setUint32(4, bytes.length - 8, true); write(8, 'WAVE'); write(12, 'fmt ');
  view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true); view.setUint32(28, sampleRate * 2, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  write(36, 'data'); view.setUint32(40, samples.length * 2, true);
  samples.forEach((sample, index) => {
    if (!Number.isFinite(sample)) throw new Error('Audio contains invalid samples');
    const clamped = Math.max(-1, Math.min(1, sample));
    view.setInt16(44 + index * 2, Math.round(clamped * (clamped < 0 ? 32768 : 32767)), true);
  });
  return bytes;
}
export async function normalizeBrowserAudio(blob: Blob): Promise<Uint8Array<ArrayBuffer>> {
  const decoder = new AudioContext();
  try {
    const decoded = await decoder.decodeAudioData(await blob.arrayBuffer());
    if (decoded.duration > 120) throw new Error('Test audio is limited to 120 seconds');
    const renderer = new OfflineAudioContext(1, Math.ceil(decoded.duration * 16_000), 16_000);
    const source = renderer.createBufferSource(); source.buffer = decoded; source.connect(renderer.destination); source.start();
    return encodePcmWav((await renderer.startRendering()).getChannelData(0));
  } finally { await decoder.close(); }
}
export async function submitSessionAudio(bytes: Uint8Array<ArrayBuffer>, target: { context?: number; process?: string; domain?: string } = {}): Promise<{ text: string }> {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(target)) if (value !== undefined && value !== '') query.set(key, String(value));
  return sessionRequest(`/audio?${query}`, { method: 'POST', headers: { 'Content-Type': 'audio/wav' }, body: new Blob([bytes], { type: 'audio/wav' }) });
}
export async function loadSessionFixture(name: string): Promise<Uint8Array<ArrayBuffer>> {
  const response = await fetch(`/__verenu_dev/fixtures/${encodeURIComponent(name)}`, { headers: { Authorization: `Bearer ${token}` }, cache: 'no-store' });
  if (!response.ok) throw new Error('Fixture could not be loaded');
  return new Uint8Array(await response.arrayBuffer());
}
