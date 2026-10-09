import { test, expect } from './fixtures.mjs';

// Regression: a transcription-only preset chosen in setup must not turn Basic
// cleanup off. Only the frontend downloaded-model inventory is a fixture;
// settings writes and the protected-setting boundary use the real backend.
// This does not verify local model downloads or inference.
test('Setup keeps Basic cleanup on after a transcription-only preset', async ({ page, session }) => {
  const previous = await session.invoke('get_all_settings');
  previous.cleanup_intensity = await session.invoke('get_setting', { key: 'cleanup_intensity' });
  previous.default_tone = await session.invoke('get_setting', { key: 'default_tone' });
  previous.setup_complete = await session.invoke('get_setting', { key: 'setup_complete' });
  previous.setup_progress = await session.invoke('get_setting', { key: 'setup_progress' });
  // Sparse synthetic stores omit defaults. Restore effective defaults where
  // the setting validator does not permit null, without touching host settings.
  const defaults = {
    setup_complete: true, setup_progress: null, cleanup_intensity: 'medium', default_tone: 'casual',
    cleanup_enabled: true, transcription_provider: 'groq', cleanup_provider: 'groq',
    transcription_models_by_provider: {}, cleanup_models_by_provider: {},
    transcription_fallback_models: [], cleanup_fallback_models: [],
    dual_transcription_enabled: false, transcription_language: 'en', appearance_mode: 'system', mute_audio: false,
  };
  try {
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 4, provider: 'local' } });
    await page.reload();

    await expect(page.getByRole('button', { name: 'Use local Transcription only' })).toBeAttached();
    await page.evaluate(async () => {
      const { localSttStore } = await import('/src/lib/localSttStore.svelte.ts');
      const { invoke } = await import('/src/lib/tauri.ts');
      const models = await invoke('list_local_stt_models');
      localSttStore.models = models.map(model => ({ ...model, is_downloaded: true, is_downloading: false }));
    });
    await expect(page.getByRole('button', { name: 'Use local Transcription only' })).toBeEnabled();
    await page.getByRole('button', { name: 'Use local Transcription only' }).click();
    await expect(page.getByRole('button', { name: 'Use local Transcription only' })).toHaveAttribute('aria-pressed', 'true');
    await page.getByRole('button', { name: 'Next', exact: true }).click();

    await page.getByRole('button', { name: /^Basic / }).click();
    await expect(page.getByRole('button', { name: /^Basic / })).toHaveAttribute('aria-pressed', 'true');

    // Basic and voice commands are gated on cleanup_enabled, so the wizard must
    // summarise Basic rather than reporting cleanup as off.
    for (let i = 0; i < 6 && !(await page.getByText('Finish setup', { exact: true }).isVisible()); i++) {
      await page.getByRole('button', { name: 'Next', exact: true }).click();
    }
    await expect(page.getByText('Finish setup', { exact: true })).toBeVisible();
    await expect(page.getByText('Basic cleanup · No AI tone', { exact: true })).toBeVisible();
    await expect(page.getByText('Cleanup off', { exact: true })).toHaveCount(0);
    const protectedSetting = page.waitForResponse(response => {
      if (!response.url().endsWith('/__verenu_dev/invoke')) return false;
      const payload = response.request().postDataJSON();
      return payload?.command === 'save_setting' && payload.args?.key === 'pause_media_during_dictation';
    });
    await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
    expect((await protectedSetting).status()).toBe(403);
    await expect(page.getByRole('alert').filter({ hasText: 'Could not save your setup choices.' })).toContainText('This setting is protected in dev sessions');
    await expect.poll(async () => (await session.invoke('get_all_settings')).cleanup_enabled).toBe(true);
    expect(await session.invoke('get_setting', { key: 'cleanup_intensity' })).toBe('rules');
    expect(await session.invoke('get_setting', { key: 'setup_complete' })).toBe(false);
  } finally {
    for (const key of [
      'setup_progress', 'setup_complete', 'cleanup_intensity', 'default_tone', 'cleanup_enabled',
      'transcription_provider', 'transcription_model', 'transcription_default_model',
      'transcription_models_by_provider', 'transcription_fallback_models', 'dual_transcription_enabled',
      'transcription_language', 'cleanup_provider', 'cleanup_model', 'cleanup_default_model',
      'cleanup_models_by_provider', 'cleanup_fallback_models', 'appearance_mode', 'mute_audio',
    ]) await session.invoke('save_setting', { key, value: previous[key] ?? defaults[key] ?? null });
    await page.reload();
  }
});
