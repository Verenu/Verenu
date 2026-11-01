export type PillSnapshot = {
  state: string;
  context: string | null;
  message: string | null;
  stage: string | null;
  revision: number;
  state_revision: number;
  context_revision: number;
  message_revision: number;
  stage_revision: number;
};

export function createPillSnapshotReceiver(handlers: {
  state: (state: string) => void;
  context: (context: string | null, state: string) => void;
  message: (message: string | null) => void;
  stage: (stage: string | null) => void;
}) {
  let revision = -1;
  let stateRevision = -1;
  let contextRevision = -1;
  let messageRevision = -1;
  let stageRevision = -1;

  return (value: unknown) => {
    if (!isPillSnapshot(value) || value.revision <= revision) return;
    revision = value.revision;
    if (value.message_revision > messageRevision) {
      messageRevision = value.message_revision;
      handlers.message(value.message);
    }
    if (value.state_revision > stateRevision) {
      stateRevision = value.state_revision;
      handlers.state(value.state || 'idle');
    }
    if (value.context_revision > contextRevision) {
      contextRevision = value.context_revision;
      handlers.context(value.context, value.state || 'idle');
    }
    if (value.stage_revision > stageRevision) {
      stageRevision = value.stage_revision;
      handlers.stage(value.stage);
    }
  };
}

function isPillSnapshot(value: unknown): value is PillSnapshot {
  if (typeof value !== 'object' || value === null) return false;
  const item = value as Record<string, unknown>;
  const optionalString = (entry: unknown) => entry === null || typeof entry === 'string';
  const revision = item.revision;
  return typeof item.state === 'string' && optionalString(item.context) &&
    optionalString(item.message) && optionalString(item.stage) &&
    typeof revision === 'number' && Number.isSafeInteger(revision) && revision >= 0 &&
    ['state_revision', 'context_revision', 'message_revision', 'stage_revision'].every((key) => {
      const n = item[key];
      return typeof n === 'number' && Number.isSafeInteger(n) && n >= 0 && n <= revision;
    });
}
