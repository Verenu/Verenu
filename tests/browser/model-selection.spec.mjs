import { test, expect, keyProviders } from './fixtures.mjs';

test('a delayed language choice preserves an agent change', async ({ page, session }) => {
  const previous = await session.invoke('get_setting', { key: 'transcription_language' });
  let release;
  let captured;
  const held = new Promise(resolve => { release = resolve; });
  const pending = new Promise(resolve => { captured = resolve; });
  try {
    await session.invoke('save_setting', { key: 'transcription_language', value: 'en' });
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command !== 'save_setting_if_unchanged' || request.args?.key !== 'transcription_language') return route.continue();
      captured();
      await held;
      await route.continue();
    });
    await page.getByRole('button', { name: 'Spoken language' }).click();
    await page.getByRole('button', { name: 'French fr' }).click();
    await pending;
    await session.invoke('save_setting', { key: 'transcription_language', value: 'de' });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['transcription_language']));
    const response = page.waitForResponse(result => result.url().includes('/__verenu_dev/invoke') && result.request().postDataJSON()?.command === 'save_setting_if_unchanged' && result.request().postDataJSON()?.args?.key === 'transcription_language');
    release();
    await response;
    await expect.poll(() => session.invoke('get_setting', { key: 'transcription_language' })).toBe('de');
    await expect(page.getByText('Settings changed outside this screen.', { exact: false })).toBeVisible();
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('save_setting', { key: 'transcription_language', value: previous ?? 'en' });
  }
});

test('an older agent cleanup refresh cannot undo a user toggle', async ({ page, session }) => {
  const previous = await session.invoke('get_setting', { key: 'cleanup_enabled' });
  let release;
  const held = new Promise(resolve => { release = resolve; });
  let received;
  const read = new Promise(resolve => { received = resolve; });
  try {
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: true });
    await page.reload();
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command !== 'get_setting' || request.args?.key !== 'cleanup_enabled') return route.continue();
      const response = await route.fetch();
      received();
      await held;
      await route.fulfill({ response });
    });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['cleanup_enabled']));
    await read;
    await page.locator('[data-debug-id="nav.settings"]').click();
    const toggle = page.locator('[data-setting-target="general-cleanup"] [role="switch"]');
    await expect(toggle).toHaveAttribute('aria-checked', 'true');
    await toggle.click();
    await page.getByRole('button', { name: 'Turn off' }).click();
    await expect.poll(() => session.invoke('get_setting', { key: 'cleanup_enabled' })).toBe(false);
    const completed = page.waitForResponse(response => response.url().includes('/__verenu_dev/invoke') && response.request().postDataJSON()?.command === 'get_setting' && response.request().postDataJSON()?.args?.key === 'cleanup_enabled');
    release();
    await completed;
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: previous ?? true });
  }
});

test('a late General settings load cannot restore old cleanup state', async ({ page, session }) => {
  const previous = await session.invoke('get_setting', { key: 'cleanup_enabled' });
  let release;
  const held = new Promise(resolve => { release = resolve; });
  let received;
  const snapshotRead = new Promise(resolve => { received = resolve; });
  try {
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: true });
    await page.reload();
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON().command !== 'get_all_settings') return route.continue();
      const response = await route.fetch();
      received();
      await held;
      await route.fulfill({ response });
    });
    await page.locator('[data-debug-id="nav.settings"]').click();
    await snapshotRead;
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: false });
    const refreshed = page.waitForResponse(response => response.url().includes('/__verenu_dev/invoke') && response.request().postDataJSON()?.command === 'get_setting' && response.request().postDataJSON()?.args?.key === 'cleanup_enabled');
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['cleanup_enabled']));
    await refreshed;
    release();
    await expect(page.locator('[data-setting-target="general-cleanup"] [role="switch"]')).toHaveAttribute('aria-checked', 'false');
    expect(await session.invoke('get_setting', { key: 'cleanup_enabled' })).toBe(false);
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: previous ?? true });
  }
});

test('a late model save response cannot restore state after an agent change', async ({ page, session }) => {
  const before = await session.invoke('get_all_settings');
  const keys = ['transcription_default_model', 'cleanup_default_model', 'transcription_fallback_models', 'cleanup_fallback_models', 'dual_transcription_enabled', 'cleanup_enabled', 'transcription_provider', 'cleanup_provider', 'transcription_model', 'cleanup_model', 'transcription_models_by_provider', 'cleanup_models_by_provider', 'model_selection_mode', 'transcription_language'];
  let release;
  const held = new Promise(resolve => { release = resolve; });
  let committed;
  const saved = new Promise(resolve => { committed = resolve; });
  try {
    await keyProviders(page, session, ['groq']);
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'groq/whisper-large-v3-turbo' });
    await session.invoke('save_setting', { key: 'transcription_language', value: 'en' });
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.models"]').click();
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON().command !== 'save_model_settings_if_unchanged') return route.fallback();
      const response = await route.fetch();
      committed();
      await held;
      await route.fulfill({ response });
    });
    await page.getByRole('button', { name: 'Use Quality', exact: true }).click();
    await saved;
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'local/parakeet-v2' });
    const refreshed = page.waitForResponse(response => response.url().includes('/__verenu_dev/invoke') && response.request().postDataJSON()?.command === 'get_effective_transcription_model');
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['transcription_default_model']));
    await expect(page.getByRole('alert')).toContainText('Model settings changed');
    await refreshed;
    const completionRefresh = page.waitForResponse(response => response.url().includes('/__verenu_dev/invoke') && response.request().postDataJSON()?.command === 'get_effective_transcription_model');
    release();
    await completionRefresh;
    await page.locator('[data-debug-id="settings.general"]').click();
    await page.getByRole('button', { name: 'Spoken language' }).click();
    await expect(page.getByRole('button', { name: 'French fr' })).toHaveCount(0);
    expect(await session.invoke('get_setting', { key: 'transcription_default_model' })).toBe('local/parakeet-v2');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    for (const key of keys) {
      if (before[key] !== undefined && before[key] !== null) await session.invoke('save_setting', { key, value: before[key] });
    }
  }
});

test('external model-related changes block preset writes until reload', async ({ page, session }) => {
  const original = await session.invoke('get_setting', { key: 'cleanup_enabled' });
  try {
    await keyProviders(page, session, ['groq']);
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.models"]').click();
    await expect(page.getByRole('region', { name: 'Cloud models' })).toBeVisible();
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: false });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['cleanup_enabled']));
    await expect(page.getByRole('alert')).toContainText('Model settings changed');
    await expect(page.getByRole('button', { name: 'Use Quality', exact: true }).locator('xpath=ancestor::*[@inert]')).toHaveCount(1);
    expect(await session.invoke('get_setting', { key: 'cleanup_enabled' })).toBe(false);
    await page.getByRole('button', { name: 'Reload model settings' }).click();
    await expect(page.getByRole('alert')).toHaveCount(0);
    expect(await session.invoke('get_setting', { key: 'cleanup_enabled' })).toBe(false);
    for (const key of ['custom_providers', 'local_model_memory_policy']) {
      await page.evaluate(async key => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', [key]), key);
      await expect(page.getByRole('alert')).toContainText('Model settings changed');
      await page.getByRole('button', { name: 'Reload model settings' }).click();
      await expect(page.getByRole('alert')).toHaveCount(0);
    }
  } finally {
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: original ?? true });
  }
});

test('agent model change updates language choices on the General screen', async ({ page, session }) => {
  const previous = await session.invoke('get_all_settings');
  try {
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'groq/whisper-large-v3-turbo' });
    await session.invoke('save_setting', { key: 'transcription_language', value: 'fr' });
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await expect(page.getByRole('button', { name: 'Spoken language' })).toContainText('French');
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'local/parakeet-v2' });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['transcription_default_model']));
    await expect.poll(() => session.invoke('get_setting', { key: 'transcription_language' })).toBe('en');
    await page.getByRole('button', { name: 'Reload settings', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Spoken language' })).toContainText('English');
    await session.invoke('save_setting', { key: 'transcription_model', value: 'groq/whisper-large-v3-turbo' });
    await session.invoke('save_setting', { key: 'transcription_default_model', value: null });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['transcription_default_model', 'transcription_model']));
    await page.getByRole('button', { name: 'Reload settings', exact: true }).click();
    await page.getByRole('button', { name: 'Spoken language' }).click();
    await expect(page.getByRole('button', { name: 'French fr' })).toBeVisible();
  } finally {
    await session.invoke('save_setting', { key: 'transcription_default_model', value: previous.transcription_default_model ?? 'groq/whisper-large-v3-turbo' });
    await session.invoke('save_setting', { key: 'transcription_model', value: previous.transcription_model ?? 'groq/whisper-large-v3-turbo' });
    await session.invoke('save_setting', { key: 'transcription_language', value: previous.transcription_language ?? 'en' });
  }
});

test('model settings warn and reload after an external change', async ({ page, session }) => {
  const previous = await session.invoke('get_setting', { key: 'transcription_default_model' });
  try {
  await keyProviders(page, session, ['groq']);
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.models"]').click();
  await expect(page.getByRole('region', { name: 'Cloud models' })).toBeVisible();
  await session.invoke('save_setting', { key: 'transcription_default_model', value: 'groq/agent-model' });
  await page.evaluate(() => window.dispatchEvent(new CustomEvent('verenu:agent-settings-changed', { detail: ['transcription_default_model'] })));
  await expect(page.getByRole('alert')).toContainText('Model settings changed outside this screen');
  await page.getByRole('button', { name: 'Reload model settings' }).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect(await session.invoke('get_setting', { key: 'transcription_default_model' })).toBe('groq/agent-model');
  } finally {
    if (previous !== null) await session.invoke('save_setting', { key: 'transcription_default_model', value: previous });
  }
});

test('other settings remain protected until the user reloads after an external change', async ({ page, session }) => {
  const previous = await session.invoke('get_setting', { key: 'history_retention' });
  try {
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.privacy"]').click();
  const retention = page.getByRole('button', { name: 'Transcription history retention' });
  await expect(retention).toBeVisible();
  await session.invoke('save_setting', { key: 'history_retention', value: '90 days' });
  await page.evaluate(() => window.dispatchEvent(new CustomEvent('verenu:agent-settings-changed', { detail: ['history_retention'] })));
  await expect(page.getByRole('alert')).toContainText('Settings changed outside this screen');
  await expect(retention.locator('xpath=ancestor::*[@inert]')).toHaveCount(1);
  await page.getByRole('button', { name: 'Reload settings', exact: true }).click();
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Transcription history retention' })).toContainText('90 days');
  } finally {
    await session.invoke('save_setting', { key: 'history_retention', value: previous ?? '30 days' });
  }
});

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
