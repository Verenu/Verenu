import { test, expect } from './fixtures.mjs';

test('cloud priorities explain their models and retain dual comparison across reload', async ({ page, session }) => {
  const provider = {
    id: 'custom:12345678-1234-1234-1234-123456789012', name: 'Synthetic provider',
    protocol: 'openai', base_url: 'http://127.0.0.1:1/v1', requires_key: false,
    supports_transcription: true, supports_cleanup: true, auth_header: null,
    extra_headers: {}, body_overrides: null,
    transcription_models: ['synthetic-primary', 'synthetic-secondary'], cleanup_models: ['synthetic-cleanup'],
  };
  const before = await session.invoke('get_all_settings');
  const mode = await session.invoke('get_setting', { key: 'model_selection_mode' });
  const keys = ['transcription_default_model', 'cleanup_default_model', 'transcription_fallback_models', 'cleanup_fallback_models', 'dual_transcription_enabled', 'cleanup_enabled', 'transcription_provider', 'cleanup_provider', 'transcription_model', 'cleanup_model', 'transcription_models_by_provider', 'cleanup_models_by_provider'];
  try {
    await session.invoke('save_setting', { key: 'custom_providers', value: [provider] });
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.models"]').click();
    await expect(page.getByRole('region', { name: 'Cloud models' })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Local AI models' })).toBeVisible();
    await expect(page.locator('.efficiency-bar')).toHaveCount(0);
    await page.getByRole('button', { name: 'Use Quality', exact: true }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'model_selection_mode' })).toBe('quality');
    await expect.poll(async () => (await session.invoke('get_all_settings')).dual_transcription_enabled).toBe(true);
    const saved = await session.invoke('get_all_settings');
    expect(saved.transcription_fallback_models).toContain(`${provider.id}/synthetic-secondary`);
    await page.getByRole('region', { name: 'Cloud models' }).getByRole('button', { name: 'Details', exact: true }).last().click();
    await expect(page.getByRole('region', { name: 'Cloud models' }).locator('.preset-details')).toContainText('synthetic-secondary');
    await page.screenshot({ path: test.info().outputPath('model-selection-details.png') });
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.models"]').click();
    await expect(page.getByRole('button', { name: 'Use Quality', exact: true })).toHaveAttribute('aria-pressed', 'true');
    expect((await session.invoke('get_all_settings')).transcription_default_model).toBe(saved.transcription_default_model);
    const localModels = await session.invoke('list_local_stt_models');
    const prepared = saved.transcription_fallback_models.some(id => localModels.some(model => model.is_downloaded && id === `local/${model.id}`));
    await expect(page.getByText(prepared ? 'Offline fallback ready' : 'Offline fallback not prepared', { exact: false })).toBeVisible();
    await page.getByRole('region', { name: 'Local AI models' }).getByRole('button', { name: 'Details', exact: true }).first().click();
    await expect(page.getByRole('button', { name: 'Prepare offline fallback' }).first()).toBeVisible();
  } finally {
    await session.invoke('save_setting', { key: 'custom_providers', value: before.custom_providers ?? [] });
    for (const key of keys) {
      if (before[key] !== undefined && before[key] !== null) await session.invoke('save_setting', { key, value: before[key] });
    }
    await session.invoke('save_setting', { key: 'model_selection_mode', value: mode ?? 'manual' });
  }
});
