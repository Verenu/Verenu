import { invoke } from './tauri';
import { saveSetting, type ProviderId } from './settings';
import { CATALOG, modelId } from './components/settings/models';

/** How long a good list stays fresh before the next Settings visit refetches. */
export const CATALOG_TTL_MS = 24 * 60 * 60 * 1000;
/** How long to wait after a failure before trying that provider again. */
export const CATALOG_RETRY_MS = 15 * 60 * 1000;
/**
 * Two misses only mean something if they're separated in time. Without this,
 * Settings-open plus a key-save seconds apart would satisfy the deprecation
 * threshold during a single provider incident.
 */
export const MISS_INTERVAL_MS = 15 * 60 * 1000;
/**
 * A 200 that drops more than this share of the ids we knew about is treated as
 * a degraded response: recorded as a success, but it advances no miss counters.
 * A provider serving a truncated list is exactly what the two-miss rule exists
 * to survive, and it doesn't arrive as an HTTP error.
 */
const SUSPICIOUS_SHRINK = 0.5;

export type MissCounter = { count: number; lastCountedAt: number };
export const CLOUD_PROVIDERS: ProviderId[] = ['groq', 'openai', 'google', 'assemblyai', 'openrouter', 'xai'];
export type ModelCapability = { label: string; tasks: ('transcription' | 'cleanup')[] };
export type ProviderModelCatalog = { ids: string[]; metadata: Record<string, ModelCapability>; warning: string | null };

export type ProviderCache = {
  ids: string[];
  /** Every id ever seen in a successful list — separates "retired" from "never existed". */
  everSeen: string[];
  /** 0 means no successful fetch has ever landed for this provider. */
  lastSuccessAt: number;
  lastAttemptAt: number;
  /** null means the last attempt succeeded. */
  lastError: string | null;
  /** Keyed by canonical `provider/model` id. */
  missing: Record<string, MissCounter>;
  metadata?: Record<string, ModelCapability>;
  warning?: string | null;
};

export type ModelCatalogCache = Partial<Record<ProviderId, ProviderCache>>;

const emptyProviderCache = (): ProviderCache => ({
  ids: [],
  everSeen: [],
  lastSuccessAt: 0,
  lastAttemptAt: 0,
  lastError: null,
  missing: {},
});

/**
 * Defensive read of a persisted blob. The Rust validator rejects malformed
 * saves, so anything wrong here came from a hand-edited settings file — drop
 * the bad parts rather than throwing on Settings open.
 */
export function mergeCatalogCache(raw: unknown): ModelCatalogCache {
  const merged: ModelCatalogCache = {};
  if (!raw || typeof raw !== 'object') return merged;

  for (const [provider, value] of Object.entries(raw as Record<string, unknown>)) {
    if (![...CLOUD_PROVIDERS, 'local'].includes(provider)) continue;
    if (!value || typeof value !== 'object') continue;
    const entry = value as Record<string, unknown>;

    const strings = (key: string): string[] =>
      Array.isArray(entry[key])
        ? (entry[key] as unknown[]).filter((v): v is string => typeof v === 'string')
        : [];
    const timestamp = (key: string): number => {
      const value = entry[key];
      return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : 0;
    };

    const missing: Record<string, MissCounter> = {};
    if (entry.missing && typeof entry.missing === 'object') {
      for (const [id, counter] of Object.entries(entry.missing as Record<string, unknown>)) {
        if (!counter || typeof counter !== 'object') continue;
        const { count, lastCountedAt } = counter as Record<string, unknown>;
        if (typeof count !== 'number' || !Number.isFinite(count) || count < 0) continue;
        missing[id] = {
          count: Math.floor(count),
          lastCountedAt:
            typeof lastCountedAt === 'number' && Number.isFinite(lastCountedAt) && lastCountedAt >= 0
              ? lastCountedAt
              : 0,
        };
      }
    }

    merged[provider as ProviderId] = {
      ids: strings('ids'),
      everSeen: strings('everSeen'),
      lastSuccessAt: timestamp('lastSuccessAt'),
      lastAttemptAt: timestamp('lastAttemptAt'),
      lastError: typeof entry.lastError === 'string' ? entry.lastError : null,
      missing,
      metadata: parseMetadata(entry.metadata),
      warning: typeof entry.warning === 'string' ? entry.warning : null,
    };
  }

  return merged;
}

export function parseMetadata(raw: unknown): Record<string, ModelCapability> {
  const result: Record<string, ModelCapability> = {};
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return result;
  for (const [id, entry] of Object.entries(raw)) {
    if (!entry || typeof entry !== 'object') continue;
    const { label, tasks } = entry as Record<string, unknown>;
    if (typeof label !== 'string' || !Array.isArray(tasks) ||
      !tasks.every((task) => task === 'transcription' || task === 'cleanup')) continue;
    result[id] = { label, tasks: tasks as ModelCapability['tasks'] };
  }
  return result;
}

/** Cached discoveries remain selectable while a provider is offline. */
export function hasSnapshot(cache: ProviderCache | undefined): boolean {
  return !!cache && cache.lastSuccessAt > 0;
}

/** True when a provider's list is complete enough to reason about absence. */
export function isTrustworthy(cache: ProviderCache | undefined): boolean {
  return !!cache && cache.lastError === null && cache.lastSuccessAt > 0;
}

export function shouldRefresh(cache: ProviderCache | undefined, now: number): boolean {
  if (!cache) return true;
  // A failed attempt earns a short cooldown, not a full day of silence — and a
  // successful one must not be retried every time Settings opens.
  if (cache.lastError !== null) return now - cache.lastAttemptAt >= CATALOG_RETRY_MS;
  return now - cache.lastSuccessAt >= CATALOG_TTL_MS;
}

/**
 * Folds a successful fetch into a provider's cache. Pure, so the counting rules
 * are testable without a clock or IPC.
 *
 * `tracked` is the set of canonical ids worth counting misses for — the user's
 * selections plus the curated catalog. Anything else is pruned so the counter
 * map can't grow without bound.
 */
export function applySuccess(
  previous: ProviderCache | undefined,
  provider: ProviderId,
  ids: string[],
  tracked: string[],
  now: number,
): ProviderCache {
  const before = previous ?? emptyProviderCache();
  const live = new Set(ids);
  const everSeen = Array.from(new Set([...before.everSeen, ...ids]));

  const knownBefore = before.ids.length;
  const stillPresent = before.ids.filter((id) => live.has(id)).length;
  const degraded =
    knownBefore > 0 && (ids.length === 0 || stillPresent < knownBefore * SUSPICIOUS_SHRINK);

  const missing: Record<string, MissCounter> = {};
  for (const id of tracked) {
    const parsed = id.startsWith(`${provider}/`) ? id.slice(provider.length + 1) : null;
    if (parsed === null) continue;
    if (live.has(parsed)) continue; // Reappeared, or never gone — counter resets.
    const prior = before.missing[id];
    if (degraded) {
      // Carry the counter forward untouched: this response can't be trusted to
      // prove absence, but it also shouldn't erase what we'd already observed.
      if (prior) missing[id] = prior;
      continue;
    }
    if (prior && now - prior.lastCountedAt < MISS_INTERVAL_MS) {
      missing[id] = prior;
      continue;
    }
    missing[id] = { count: (prior?.count ?? 0) + 1, lastCountedAt: now };
  }

  return {
    // A truncated 200 must not make the picker drop known models.
    ids: degraded ? Array.from(new Set([...before.ids, ...ids])) : ids,
    everSeen,
    lastSuccessAt: now,
    lastAttemptAt: now,
    lastError: degraded ? 'The provider returned an incomplete list. Keeping cached models.' : null,
    missing,
    metadata: before.metadata,
    warning: before.warning,
  };
}

/** A failed fetch records the attempt and touches nothing else. */
export function applyFailure(
  previous: ProviderCache | undefined,
  error: string,
  now: number,
): ProviderCache {
  const before = previous ?? emptyProviderCache();
  return { ...before, lastAttemptAt: now, lastError: error };
}

// ── Store ──────────────────────────────────────────────────────────────────

export const modelCatalogStore = $state<{ cache: ModelCatalogCache; refreshing: Partial<Record<ProviderId, boolean>> }>({ cache: {}, refreshing: {} });

/** One in-flight request per provider, so overlapping triggers coalesce. */
const inFlight = new Map<ProviderId, Promise<void>>();
/** Serializes persistence: a per-provider refresh must never clobber another's entry. */
let writeChain: Promise<unknown> = Promise.resolve();
let hydrated = false;
let hydration: Promise<void> | undefined;

export function hydrateCatalogCache(raw: unknown) {
  // An in-flight refresh can finish while Models loads settings. Keep its
  // newer result instead of replacing it with an older persisted snapshot.
  const saved = mergeCatalogCache(raw);
  for (const provider of CLOUD_PROVIDERS) {
    const current = modelCatalogStore.cache[provider];
    if (current && current.lastAttemptAt >= (saved[provider]?.lastAttemptAt ?? 0)) saved[provider] = current;
  }
  modelCatalogStore.cache = saved;
  hydrated = true;
}

async function ensureHydrated() {
  if (hydrated) return;
  hydration ??= invoke<unknown>('get_setting', { key: 'provider_model_cache' }).then(hydrateCatalogCache).finally(() => { hydration = undefined; });
  await hydration;
}

function persist() {
  // Always writes the whole object, never one provider's slice.
  const snapshot = JSON.parse(JSON.stringify(modelCatalogStore.cache)) as ModelCatalogCache;
  writeChain = writeChain
    .then(() => saveSetting('provider_model_cache', snapshot))
    .catch((error) => console.warn('Failed to persist model catalog cache', error));
  return writeChain.then(() => undefined);
}

export function refreshCatalog(provider: ProviderId, tracked: string[], now = Date.now()) {
  const pending = inFlight.get(provider);
  if (pending) return pending;

  modelCatalogStore.refreshing[provider] = true;
  let loaded = false;
  const request = ensureHydrated().then(() => {
    loaded = true;
    return invoke<ProviderModelCatalog>('get_provider_model_catalog', { provider });
  })
    .then((catalog) => {
      const updated = applySuccess(
        modelCatalogStore.cache[provider],
        provider,
        catalog.ids,
        trackedIds([...tracked, ...Object.keys(modelCatalogStore.cache[provider]?.missing ?? {})], CATALOG),
        now,
      );
      // Retain capabilities on a metadata outage, including across restarts.
      updated.metadata = { ...updated.metadata, ...parseMetadata(catalog.metadata) };
      updated.warning = catalog.warning;
      modelCatalogStore.cache[provider] = updated;
    })
    .catch((error) => {
      if (!loaded) {
        console.warn('Could not load the saved model catalog before refreshing', error);
        return;
      }
      modelCatalogStore.cache[provider] = applyFailure(
        modelCatalogStore.cache[provider],
        String(error),
        now,
      );
    })
    .then(() => {
      // Never replace a saved cache with an empty snapshot after a failed read.
      if (loaded) return persist();
    })
    .finally(() => {
      inFlight.delete(provider);
      modelCatalogStore.refreshing[provider] = false;
    });

  inFlight.set(provider, request);
  return request;
}

/** Refreshes every keyed provider whose cache has gone stale. */
export async function refreshStaleCatalogs(
  apiKeyStatus: Record<ProviderId, boolean>,
  tracked: string[],
  now = Date.now(),
) {
  // Decide freshness after loading the persisted cache. Otherwise opening
  // Settings launches requests before hydration and refetches on every visit.
  await ensureHydrated();
  return Promise.all(
    CLOUD_PROVIDERS
      .filter((provider) => apiKeyStatus[provider] || provider === 'openrouter')
      .filter((provider) => shouldRefresh(modelCatalogStore.cache[provider], now))
      .map((provider) => refreshCatalog(provider, tracked, now)),
  );
}

/** Canonical ids worth tracking misses for: the user's picks plus the catalog. */
export function trackedIds(selected: string[], curated: { provider: ProviderId; id: string }[]) {
  return Array.from(
    new Set([...selected, ...curated.map((entry) => modelId(entry.provider, entry.id))]),
  );
}
