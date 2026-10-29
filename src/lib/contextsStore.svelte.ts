import { invoke } from './tauri';
import { classifyIpcError } from './errors';
import type { Context, ContextSubApp, ContextTarget, ContextWebsiteTarget } from './stores';
import type { SubAppCapture } from './subApps';

export const EVERYWHERE_ID = 1;

export const contextsStore = $state({
  contexts: [] as Context[],
  targets: [] as ContextTarget[],
  websites: [] as ContextWebsiteTarget[],
  subApps: [] as ContextSubApp[],
  subAppSheet: null as null | { mode: 'capture'; capture: SubAppCapture },
  selectedId: EVERYWHERE_ID,
  selectDir: 1 as 1 | -1,
  loaded: false,
  error: '',
  modalRequest: null as null | { mode: 'create' } | { mode: 'edit'; id: number },
});

export function orderedContexts(list: Context[]): Context[] {
  const pinned = list
    .filter((context) => context.pinned_at)
    .sort((a, b) => (b.pinned_at ?? '').localeCompare(a.pinned_at ?? '') || b.id - a.id);
  const rest = list.filter((context) => !context.pinned_at);
  return [...pinned, ...rest];
}

export function selectContext(id: number): void {
  if (id === contextsStore.selectedId) return;
  const order = orderedContexts(contextsStore.contexts);
  const from = order.findIndex((context) => context.id === contextsStore.selectedId);
  const to = order.findIndex((context) => context.id === id);
  contextsStore.selectDir = from === -1 || to === -1 || to >= from ? 1 : -1;
  contextsStore.selectedId = id;
}

export function isPinned(context: Context): boolean {
  return Boolean(context.pinned_at);
}

export function selectedContext(): Context | null {
  return (
    contextsStore.contexts.find((context) => context.id === contextsStore.selectedId) ??
    contextsStore.contexts[0] ??
    null
  );
}

let inFlight: Promise<void> | null = null;

export function loadContexts(force = false): Promise<void> {
  if (inFlight) return force ? inFlight.then(() => loadContexts(true)) : inFlight;
  if (contextsStore.loaded && !force) return Promise.resolve();
  inFlight = (async () => {
    try {
      const [contexts, targets, websites, subApps] = await Promise.all([
        invoke<Context[]>('get_contexts'),
        invoke<ContextTarget[]>('get_context_targets', { contextId: null }),
        invoke<ContextWebsiteTarget[]>('get_context_websites', { contextId: null }),
        invoke<ContextSubApp[]>('get_sub_apps'),
      ]);
      contextsStore.contexts = contexts ?? [];
      contextsStore.targets = targets ?? [];
      contextsStore.websites = websites ?? [];
      contextsStore.subApps = subApps ?? [];
      contextsStore.error = '';
      contextsStore.loaded = true;
      if (!contextsStore.contexts.some((context) => context.id === contextsStore.selectedId)) {
        contextsStore.selectedId =
          contextsStore.contexts.find((context) => context.is_everywhere)?.id ??
          contextsStore.contexts[0]?.id ??
          EVERYWHERE_ID;
      }
    } catch (error) {
      contextsStore.error = classifyIpcError(error).message;
    } finally {
      inFlight = null;
    }
  })();
  return inFlight;
}

export function compactAge(iso: string): string {
  const parsed = Date.parse(/(?:[Z+]|-\d{2}:?\d{2})$/.test(iso.slice(10)) ? iso : `${iso.replace(' ', 'T')}Z`);
  if (Number.isNaN(parsed)) return '';
  const seconds = Math.max(0, Math.floor((Date.now() - parsed) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  const weeks = Math.floor(days / 7);
  if (days < 30) return `${weeks}w`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo`;
  return `${Math.max(1, Math.floor(days / 365))}y`;
}
