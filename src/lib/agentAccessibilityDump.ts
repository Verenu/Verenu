/** Compact developer-only accessibility dump for T3 Code SnapShots. */

export const RUIN_ACCESSIBILITY_EVENT = 'verenu:ruin-accessibility-changed';
export const DUMP_REGION_ATTR = 'data-verenu-ax-dump-region';
export const DUMP_ANNOTATION_ATTR = 'data-verenu-ax-dump';
export const DEFAULT_WINDOW_TITLE = 'Verenu';
/** T3 truncates the tree. Keep the dump small enough to survive. */
export const MAX_DUMP_CHARS = 3_200;
export const RECENT_EVENT_CAP = 12;

const STARTED_AT_MS = Date.now();

const REDACT_KEY = /api_key_|password|secret|token|credential/i;
const REDACT_EXACT = new Set([
  'clipboard_phrase',
  'cleanup_prompt_override',
  'key_groq',
  'key_openai',
  'key_google',
  'key_assemblyai',
]);

const PRIORITY_SETTINGS = [
  'transcription_provider',
  'transcription_model',
  'transcription_language',
  'cleanup_provider',
  'cleanup_model',
  'cleanup_enabled',
  'dual_transcription_enabled',
  'microphone_device',
  'hotkey',
  'noise_reduction',
  'mic_gain',
  'exclusive_mic',
  'pause_media_during_dictation',
  'mic_mute_button_dictation',
  'local_model_memory_policy',
  'appearance_mode',
  'setup_complete',
  'advanced_model_ui',
  'auto_learn_enabled',
  'app_context_hint',
  'ruin_accessibility',
] as const;

export type DumpWindowKind = 'main' | 'pill';

export type BuildIdentity = {
  commit: string;
  branch: string;
  dirty: boolean;
  timestamp: string;
};

export type DumpEvent = {
  at: number;
  name: string;
  detail?: string;
};

export type AgentDumpSnapshot = {
  windowKind: DumpWindowKind;
  version: string;
  extras: Record<string, unknown>;
  settings: Record<string, unknown>;
  errors: string[];
  inventory: string[];
  recent?: DumpEvent[];
  build?: BuildIdentity;
};

let recentEvents: DumpEvent[] = [];

export function readBuildIdentity(): BuildIdentity {
  const sha = typeof __VERENU_GIT_SHA__ === 'string' ? __VERENU_GIT_SHA__ : 'unknown';
  const branch = typeof __VERENU_GIT_BRANCH__ === 'string' ? __VERENU_GIT_BRANCH__ : 'unknown';
  const dirty = typeof __VERENU_GIT_DIRTY__ === 'boolean' ? __VERENU_GIT_DIRTY__ : false;
  const timestamp = typeof __VERENU_BUILD_TIME__ === 'string' ? __VERENU_BUILD_TIME__ : 'unknown';
  return { commit: sha, branch, dirty, timestamp };
}

export function pushDumpEvent(name: string, detail?: string): DumpEvent[] {
  const at = Date.now();
  const last = recentEvents[recentEvents.length - 1];
  const wait = last ? `+${at - last.at}ms` : undefined;
  const parts = [detail, wait].filter(Boolean).join(' ');
  recentEvents = [...recentEvents.slice(-(RECENT_EVENT_CAP - 1)), {
    at,
    name,
    detail: parts || undefined,
  }];
  return recentEvents;
}

export function getDumpEvents(): DumpEvent[] {
  return recentEvents;
}

export function formatUptime(nowMs = Date.now(), startedAtMs = STARTED_AT_MS): string {
  const elapsed = Math.max(0, nowMs - startedAtMs);
  const seconds = Math.floor(elapsed / 1000);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = seconds % 60;
  if (hours > 0) return `${hours}h ${minutes}m ${rest}s`;
  if (minutes > 0) return `${minutes}m ${rest}s`;
  return `${rest}s`;
}

export function redactValue(value: unknown): string {
  if (value == null || value === '') return '[redacted empty]';
  if (typeof value === 'string') return `[redacted len=${value.length}]`;
  if (Array.isArray(value)) return `[redacted items=${value.length}]`;
  return '[redacted]';
}

export function shouldRedactSettingKey(key: string): boolean {
  return REDACT_EXACT.has(key) || REDACT_KEY.test(key);
}

export function redactSettings(raw: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(raw)) {
    out[key] = shouldRedactSettingKey(key) ? redactValue(value) : value;
  }
  return out;
}

function compactValue(value: unknown): string {
  if (value == null) return 'null';
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }
  if (Array.isArray(value)) {
    if (value.every((item) => typeof item === 'string' || typeof item === 'number')) {
      return value.join('+');
    }
    return `[${value.length}]`;
  }
  return '{…}';
}

function clock(at: number): string {
  return new Date(at).toISOString().slice(11, 19);
}

export function compactDumpTitle(snapshot: AgentDumpSnapshot): string {
  const page = String(snapshot.extras.page ?? snapshot.windowKind);
  const build = snapshot.build ?? readBuildIdentity();
  const sha = build.commit !== 'unknown' ? build.commit : '';
  return `Verenu DUMP v${snapshot.version || '?'} ${sha} ${snapshot.windowKind} ${page} up ${formatUptime()}`.replace(/\s+/g, ' ').trim();
}

function pickSettings(settings: Record<string, unknown>): string[] {
  const redacted = redactSettings(settings);
  const lines: string[] = [];
  for (const key of PRIORITY_SETTINGS) {
    if (!(key in redacted)) continue;
    lines.push(`${key}=${compactValue(redacted[key])}`);
  }
  return lines;
}

export function buildAgentDump(snapshot: AgentDumpSnapshot): string {
  const build = snapshot.build ?? readBuildIdentity();
  const extras = snapshot.extras;
  const lastError = snapshot.errors[snapshot.errors.length - 1] ?? 'none';
  const recent = snapshot.recent ?? recentEvents;
  const lines: string[] = [
    'VERENU_AX_DUMP',
    `build.commit=${build.commit} branch=${build.branch} dirty=${build.dirty} time=${build.timestamp}`,
    `runtime.window=${snapshot.windowKind} version=${snapshot.version || 'unknown'} up=${formatUptime()} page=${compactValue(extras.page ?? snapshot.windowKind)} settingsOpen=${compactValue(extras.settingsOpen)} section=${compactValue(extras.settingsSection)}`,
    `state.online=${compactValue(extras.isOnline)} apiHealthy=${compactValue(extras.apiHealthy)} pill=${compactValue(extras.pillState)} setup=${compactValue(extras.setupComplete)}`,
    `pipeline=${compactValue(extras.pillState ?? 'idle')} last_error=${lastError}`,
  ];

  const diagnostics = extras.diagnostics && typeof extras.diagnostics === 'object'
    ? extras.diagnostics as Record<string, unknown>
    : {};
  lines.push(
    `diag.cpu=${compactValue(diagnostics.cpu)} resident=${compactValue(diagnostics.residentBytes)} active_traces=${compactValue(diagnostics.activeTraces)} failures=${compactValue(diagnostics.recentFailures)} profiler=${compactValue(diagnostics.profiler)} dropped=${compactValue(diagnostics.dropped)}`,
  );
  lines.push(`diag.hot_ops=${compactValue(diagnostics.hottestOperations)} fingerprints=${compactValue(diagnostics.failureFingerprints)}`);

  const stt = extras.localStt && typeof extras.localStt === 'object' ? extras.localStt as Record<string, unknown> : {};
  const llm = extras.localLlm && typeof extras.localLlm === 'object' ? extras.localLlm as Record<string, unknown> : {};
  const sync = extras.sync && typeof extras.sync === 'object' ? extras.sync as Record<string, unknown> : {};
  const platform = extras.platform && typeof extras.platform === 'object' ? extras.platform as Record<string, unknown> : {};

  lines.push(
    `providers.stt=${compactValue(snapshot.settings.transcription_provider)}/${compactValue(snapshot.settings.transcription_model)} cleanup=${compactValue(snapshot.settings.cleanup_provider)}/${compactValue(snapshot.settings.cleanup_model)} mic=${compactValue(snapshot.settings.microphone_device)} hotkey=${compactValue(snapshot.settings.hotkey)}`,
  );
  lines.push(
    `local.stt=${compactValue(stt.current)} loaded=${compactValue(stt.loaded)} llm=${compactValue(llm.current)} loaded=${compactValue(llm.loaded)} runtime=${compactValue(llm.runtimeInstalled)}`,
  );
  lines.push(
    `sync.enabled=${compactValue(extras.syncEnabled)} listener=${compactValue(sync.listenerActive)} peers=${compactValue(sync.peerCount)} pairing=${compactValue(sync.pairingPhase)} online=${compactValue(extras.isOnline)}`,
  );
  lines.push(
    `platform.win=${compactValue(platform.isWindows)} mac=${compactValue(platform.isMac)} android=${compactValue(platform.isAndroid)} reducedMotion=${compactValue(extras.reducedMotion)}`,
  );

  const contextCount = extras.contextCount ?? (extras.contexts && typeof extras.contexts === 'object' ? (extras.contexts as { count?: unknown }).count : undefined);
  const selected = extras.contextSelected ?? (extras.contexts && typeof extras.contexts === 'object' ? (extras.contexts as { selectedId?: unknown }).selectedId : undefined);
  lines.push(`contexts.count=${compactValue(contextCount)} selected=${compactValue(selected)} snippets=${compactValue(extras.snippetCount)} dictionary=${compactValue(extras.dictionaryCount)}`);

  lines.push('recent:');
  if (recent.length === 0) {
    lines.push('  (none)');
  } else {
    for (const event of recent) {
      lines.push(`  ${clock(event.at)} ${event.name}${event.detail ? ` ${event.detail}` : ''}`);
    }
  }

  if (snapshot.errors.length > 0) {
    lines.push('errors:');
    for (const error of snapshot.errors.slice(-4)) {
      lines.push(`  ${error}`);
    }
  }

  lines.push('settings:');
  const settingLines = pickSettings(snapshot.settings);
  if (settingLines.length === 0) {
    lines.push('  (none)');
  } else {
    for (const line of settingLines) lines.push(`  ${line}`);
  }

  if (snapshot.inventory.length > 0) {
    lines.push('ui:');
    for (const line of snapshot.inventory.slice(0, 12)) lines.push(`  ${line}`);
  }

  const text = lines.join('\n');
  if (text.length <= MAX_DUMP_CHARS) return text;
  return `${text.slice(0, MAX_DUMP_CHARS)}\n…truncated ${text.length - MAX_DUMP_CHARS} chars`;
}

function semanticClasses(className: string): string[] {
  return className
    .split(/\s+/)
    .filter((token) => token && !token.startsWith('svelte-') && !/^[A-Z][A-Za-z0-9_-]{6,}$/.test(token));
}

export function describeDumpElement(el: Element): string {
  const parts: string[] = [];
  const debugId = el.getAttribute('data-debug-id');
  const setting = el.getAttribute('data-setting-target')
    ?? el.closest?.('[data-setting-target]')?.getAttribute('data-setting-target');
  if (debugId) parts.push(`id=${debugId}`);
  if (setting) parts.push(`setting=${setting}`);
  const className = typeof el.className === 'string' ? el.className.trim() : '';
  const classes = semanticClasses(className);
  const component = classes.find((token) => !['on', 'active', 'open', 'disabled'].includes(token));
  if (component) parts.push(`component=${component}`);
  if (classes.includes('active')) parts.push('state=active');
  const checked = el.getAttribute('aria-checked');
  if (checked != null) parts.push(`value=${checked}`);
  for (const name of ['aria-expanded', 'aria-selected', 'aria-current', 'aria-pressed'] as const) {
    const value = el.getAttribute(name);
    if (value != null) parts.push(`${name.replace('aria-', '')}=${value}`);
  }
  if ('disabled' in el && (el as HTMLButtonElement).disabled) parts.push('disabled');
  if (parts.length === 0) {
    const tag = el.tagName.toLowerCase();
    parts.push(`tag=${tag}`);
  }
  return parts.join(' ');
}

export function collectDomInventory(root: ParentNode = document): string[] {
  const lines: string[] = [];
  const switches = root.querySelectorAll('[role="switch"]');
  switches.forEach((el) => {
    const setting = el.closest('[data-setting-target]')?.getAttribute('data-setting-target')
      ?? el.getAttribute('data-debug-id')
      ?? el.getAttribute('aria-label')
      ?? '';
    lines.push(`switch ${setting} value=${el.getAttribute('aria-checked')}`);
  });
  const current = root.querySelector('[data-debug-id].active, .nav-item.active, .settings-nav-item.active, .ctx-row.active');
  if (current instanceof Element) {
    lines.push(`active ${describeDumpElement(current)}`);
  }
  return lines.slice(0, 16);
}

const ANNOTATE_SELECTOR = [
  'button',
  'a',
  'input',
  'select',
  'textarea',
  'h1',
  'h2',
  '[role="switch"]',
  '[data-setting-target]',
  '[data-debug-id]',
  '[aria-current]',
].join(',');

export function annotateAccessibilityDump(root: ParentNode = document): void {
  const nodes = root.querySelectorAll(ANNOTATE_SELECTOR);
  for (const el of nodes) {
    if (el.closest(`[${DUMP_REGION_ATTR}]`)) continue;
    const extra = describeDumpElement(el);
    if (!extra) continue;
    if (!el.hasAttribute(DUMP_ANNOTATION_ATTR)) {
      el.setAttribute(DUMP_ANNOTATION_ATTR, el.getAttribute('aria-description') ?? '');
    }
    const original = el.getAttribute(DUMP_ANNOTATION_ATTR) ?? '';
    const next = original ? `${original}. ${extra}` : extra;
    if (el.getAttribute('aria-description') !== next) {
      el.setAttribute('aria-description', next);
    }
  }
}

export function restoreAccessibilityDump(root: ParentNode = document): void {
  const nodes = root.querySelectorAll(`[${DUMP_ANNOTATION_ATTR}]`);
  for (const el of nodes) {
    const original = el.getAttribute(DUMP_ANNOTATION_ATTR);
    if (original) el.setAttribute('aria-description', original);
    else el.removeAttribute('aria-description');
    el.removeAttribute(DUMP_ANNOTATION_ATTR);
  }
}

export function applyDumpWindowTitle(title: string): string {
  if (typeof document === 'undefined') return title;
  document.title = title;
  document.documentElement.setAttribute('aria-label', title);
  return title;
}

export function restoreDumpWindowTitle(): void {
  if (typeof document === 'undefined') return;
  document.title = DEFAULT_WINDOW_TITLE;
  document.documentElement.removeAttribute('aria-label');
}
