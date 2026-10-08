export const HOME_READINESS_SYNC_EVENT = 'verenu:sync-data-changed';

export type ReadinessSyncPayload = { tables?: string[] };
export type ReadinessSyncEvent = { payload: ReadinessSyncPayload };
export type ReadinessSyncListener = (
  eventName: typeof HOME_READINESS_SYNC_EVENT,
  handler: (event: ReadinessSyncEvent) => void,
) => Promise<() => void>;

const READINESS_TABLES = new Set(['settings', 'contexts']);

export function affectsHomeReadiness(tables?: readonly string[]): boolean {
  return tables?.some((table) => READINESS_TABLES.has(table)) ?? false;
}

/** Subscribe to relevant sync changes and dispose safely if mounting races registration. */
export function listenForHomeReadinessSyncChanges(
  listen: ReadinessSyncListener,
  refresh: () => void,
): () => void {
  let active = true;
  let unlisten: (() => void) | undefined;

  void listen(HOME_READINESS_SYNC_EVENT, (event) => {
    if (active && affectsHomeReadiness(event.payload?.tables)) refresh();
  }).then((stop) => {
    if (active) unlisten = stop;
    else stop();
  }).catch(() => {});

  return () => {
    if (!active) return;
    active = false;
    unlisten?.();
    unlisten = undefined;
  };
}
