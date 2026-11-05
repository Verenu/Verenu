import { invoke, isTauriRuntime } from './tauri';

/**
 * Forwards webview errors to the native session log, so triage can see frontend
 * failures next to backend ones. The native side accepts only these sources and
 * bounds the text, so this module only has to keep the volume sane.
 */

export type FrontendLogSource = 'window.error' | 'unhandledrejection' | 'console.error' | 'console.warn';

export const FRONTEND_LOG_LIMIT = 50;
export const FRONTEND_LOG_WINDOW_MS = 60_000;

/** One line of text for a value passed to console.error or thrown into the window. */
export function formatForwardedValue(value: unknown): string {
  if (value instanceof Error) return `${value.name}: ${value.message}`;
  if (typeof value === 'string') return value;
  try {
    const text = JSON.stringify(value);
    return text === undefined ? String(value) : text;
  } catch {
    return String(value);
  }
}

export function formatForwardedArgs(args: unknown[]): string {
  return args.map(formatForwardedValue).join(' ');
}

/**
 * Allows at most `limit` forwarded messages in any `windowMs` span. The window slides, so a
 * burst at the end of one window and the start of the next still counts against the limit.
 * Each allowed send counts, including sends that re-enter the forwarder, so the limiter also
 * bounds any feedback loop.
 */
export function createForwardLimiter(limit = FRONTEND_LOG_LIMIT, windowMs = FRONTEND_LOG_WINDOW_MS, now = Date.now) {
  const sent: number[] = [];
  return (): boolean => {
    const at = now();
    while (sent.length > 0 && at - sent[0] >= windowMs) sent.shift();
    if (sent.length >= limit) return false;
    sent.push(at);
    return true;
  };
}

/**
 * Installs the forwarders once. `console.error` and `console.warn` keep printing to the
 * devtools console. A failing forward is ignored, and the rate limiter bounds any loop
 * that a failing forward could start.
 */
export function installFrontendLogForwarding(
  send: (level: 'error' | 'warn', source: FrontendLogSource, message: string) => Promise<unknown> = (level, source, message) =>
    invoke('frontend_log', { level, source, message }),
  target: Pick<Window, 'addEventListener'> & { console: Console } = window as Window & { console: Console },
): void {
  const allow = createForwardLimiter();

  const forward = (level: 'error' | 'warn', source: FrontendLogSource, message: string) => {
    if (!allow()) return;
    try {
      void Promise.resolve(send(level, source, message)).catch(() => {});
    } catch {
      // Forwarding is best effort. The devtools console still shows the original message.
    }
  };

  target.addEventListener('error', (event: ErrorEvent) => {
    const where = event.filename ? ` at ${event.filename}:${event.lineno}:${event.colno}` : '';
    forward('error', 'window.error', `${event.message || 'error'}${where}`);
  });
  target.addEventListener('unhandledrejection', (event: PromiseRejectionEvent) => {
    forward('error', 'unhandledrejection', formatForwardedValue(event.reason));
  });

  const originalError = target.console.error.bind(target.console);
  const originalWarn = target.console.warn.bind(target.console);
  target.console.error = (...args: unknown[]) => {
    originalError(...args);
    forward('error', 'console.error', formatForwardedArgs(args));
  };
  target.console.warn = (...args: unknown[]) => {
    originalWarn(...args);
    forward('warn', 'console.warn', formatForwardedArgs(args));
  };
}

/** Installs the forwarders only inside the desktop app, where the native command exists. */
export function installFrontendLogForwardingIfTauri(): void {
  if (isTauriRuntime()) installFrontendLogForwarding();
}
