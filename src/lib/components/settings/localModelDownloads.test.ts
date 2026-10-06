import { describe, expect, it, vi } from 'vitest';
import { localModelDownloads } from './localModelDownloads';

const { stt, llm } = vi.hoisted(() => ({
  stt: { download: vi.fn(), cancel: vi.fn(), delete: vi.fn() },
  llm: { download: vi.fn(), cancel: vi.fn(), delete: vi.fn() },
}));
vi.mock('../../localSttStore.svelte', () => ({
  downloadLocalModel: stt.download, cancelLocalModelDownload: stt.cancel, deleteLocalModel: stt.delete,
}));
vi.mock('../../localLlmStore.svelte', () => ({
  downloadLocalLlmModel: llm.download, cancelLocalLlmModelDownload: llm.cancel, deleteLocalLlmModel: llm.delete,
}));

describe('preset download dispatch', () => {
  it.each(['download', 'cancel', 'delete'] as const)('routes %s to the correct manager and preserves results', async action => {
    for (const [task, manager, other] of [['transcription', stt, llm], ['cleanup', llm, stt]] as const) {
      stt[action].mockReset(); llm[action].mockReset();
      manager[action].mockResolvedValue(false);
      expect(await localModelDownloads[task][action]('public-model')).toBe(false);
      expect(manager[action]).toHaveBeenCalledExactlyOnceWith('public-model');
      expect(other[action]).not.toHaveBeenCalled();
    }
  });
});
