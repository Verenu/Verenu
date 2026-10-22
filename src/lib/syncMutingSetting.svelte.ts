import { loadSettingsSnapshot, saveSetting } from './settings';

type SyncMutingPersistence = {
  load: () => Promise<boolean | null | undefined>;
  save: (enabled: boolean) => Promise<unknown>;
};

const persistence: SyncMutingPersistence = {
  load: async () => (await loadSettingsSnapshot()).sync_muting_enabled,
  save: (enabled) => saveSetting('sync_muting_enabled', enabled),
};

export class SyncMutingSetting {
  enabled = $state(false);
  loaded = $state(false);
  loadFailed = $state(false);
  saving = $state(false);
  saveFailed = $state(false);
  flashError = $state(false);

  private readonly persistence: SyncMutingPersistence;
  private loading = false;

  constructor(persistenceOverride: SyncMutingPersistence = persistence) {
    this.persistence = persistenceOverride;
  }

  get disabled(): boolean {
    return !this.loaded || this.saving;
  }

  async load(): Promise<void> {
    if (this.loading || this.loaded) return;
    this.loading = true;
    this.loadFailed = false;
    try {
      this.enabled = (await this.persistence.load()) ?? false;
      this.loaded = true;
    } catch (err) {
      this.loadFailed = true;
      console.error('Failed to load sync muting setting:', err);
    } finally {
      this.loading = false;
    }
  }

  async setEnabled(value: boolean): Promise<void> {
    if (this.disabled) return;
    const previous = this.enabled;
    this.enabled = value;
    this.saving = true;
    this.saveFailed = false;
    try {
      await this.persistence.save(value);
    } catch (err) {
      this.enabled = previous;
      this.saveFailed = true;
      this.flashError = true;
      console.error('save sync_muting_enabled failed:', err);
    } finally {
      this.saving = false;
    }
  }
}
