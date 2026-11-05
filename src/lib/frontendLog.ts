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

/** Allows `limit` forwarded messages per window, so a failure loop cannot flood the log. */
export function createForwardLimiter(limit = FRONTEND_LOG_LIMIT, windowMs = FRONTEND_LOG_WINDOW_MS, now = Date.now) {
  let windowStart = now();
  let count = 0;
  return (): boolean => {
    const at = now();
    if (at - windowStart >= windowMs) {
      windowStart = at;
      count = 0;
    }
    if (count >= limit) return false;
    count += 1;
    return true;
  };
}

/**
 * Installs the forwarders once. `console.error` and `console.warn` keep printing to the
 * devtools console, and each forwarded call is guarded so a failing forward cannot
 * recurse into another forward.
 */
export function installFrontendLogForwarding(
  send: (level: 'error' | 'warn', source: FrontendLogSource, message: string) => Promise<unknown> = (level, source, message) =>
    invoke('frontend_log', { level, source, message }),
  target: Pick<Window, 'addEventListener'> & { console: Console } = window as Window & { console: Console },
): void {
  const allow = createForwardLimiter();
  let forwarding = false;

  const forward = (level: 'error' | 'warn', source: FrontendLogSource, message: string) => {
    if (forwarding || !allow()) return;
    forwarding = true;
    try {
      void Promise.resolve(send(level, source, message)).catch(() => {});
    } catch {
      // Forwarding is best effort. The devtools console still shows the original message.
    } finally {
      forwarding = false;
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
