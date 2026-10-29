import { test, expect } from './fixtures.mjs';

test('an accent choice cannot overwrite a concurrent agent change', async ({ page, session }) => {
  const previous = await session.invoke('get_setting', { key: 'accent_color' });
  let release;
  let captured;
  const held = new Promise(resolve => { release = resolve; });
  const pending = new Promise(resolve => { captured = resolve; });
  try {
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'save_model_settings_if_unchanged') return route.continue();
      captured();
      await held;
      await route.continue();
    });
    await page.evaluate(async () => {
      const actions = await import('/src/lib/appearanceActions.ts');
      window.__accentResult = actions.persistAccentColor('#D2637A');
    });
    await pending;
    await session.invoke('save_setting', { key: 'accent_color', value: '#4F7FD8' });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['accent_color']));
    release();
    expect(await page.evaluate(() => window.__accentResult)).toBe(false);
    expect(await session.invoke('get_setting', { key: 'accent_color' })).toBe('#4F7FD8');
    await expect.poll(() => page.evaluate(async () => (await import('/src/lib/stores.svelte.ts')).appStore.accentColor)).toBe('#4F7FD8');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('save_setting', { key: 'accent_color', value: previous ?? null });
  }
});

test('applying a palette preserves a concurrent agent appearance change', async ({ page, session }) => {
  const keys = ['appearance_mode', 'accent_color', 'custom_theme', 'custom_themes'];
  const before = Object.fromEntries(await Promise.all(keys.map(async key => [key, await session.invoke('get_setting', { key })])));
  let release;
  let captured;
  const held = new Promise(resolve => { release = resolve; });
  const pending = new Promise(resolve => { captured = resolve; });
  const agentTheme = { background: '#202020', foreground: '#EEEEEE', sidebar: null, surface: null };
  try {
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'save_model_settings_if_unchanged') return route.continue();
      captured();
      await held;
      await route.continue();
    });
    await page.evaluate(async () => {
      const actions = await import('/src/lib/appearanceActions.ts');
      window.__paletteResult = actions.applyPalette({ background: '#FFFFFF', foreground: '#111111', sidebar: null, surface: null });
    });
    await pending;
    await session.invoke('save_setting', { key: 'custom_theme', value: agentTheme });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['custom_theme']));
    release();
    expect(await page.evaluate(() => window.__paletteResult)).toBe(false);
    expect(await session.invoke('get_setting', { key: 'custom_theme' })).toEqual(agentTheme);
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    for (const key of keys) await session.invoke('save_setting', { key, value: before[key] ?? (key === 'appearance_mode' ? 'system' : key === 'custom_themes' ? [] : null) });
  }
});

test('agent appearance and legacy changes update the running app', async ({ page, session }) => {
  const keys = ['appearance_mode', 'accent_color', 'legacy_features_enabled'];
  const before = Object.fromEntries(await Promise.all(keys.map(async key => [key, await session.invoke('get_setting', { key })])));
  try {
    await page.evaluate(async () => {
      window.__agentAccentEvents = [];
      await (await import('/src/lib/tauri.ts')).listen('verenu:accent-color-changed', event => window.__agentAccentEvents.push(event.payload));
    });
    await session.invoke('save_setting', { key: 'appearance_mode', value: 'dark' });
    await session.invoke('save_setting', { key: 'accent_color', value: '#D2637A' });
    await session.invoke('save_setting', { key: 'legacy_features_enabled', value: true });
    await page.evaluate(async keys => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', keys), keys);
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await expect.poll(() => page.evaluate(async () => {
      const { appStore } = await import('/src/lib/stores.svelte.ts');
      return [appStore.accentColor, appStore.legacyFeaturesEnabled];
    })).toEqual(['#D2637A', true]);
    await expect.poll(() => page.evaluate(() => window.__agentAccentEvents)).toContain('#D2637A');
  } finally {
    for (const key of keys) await session.invoke('save_setting', { key, value: before[key] ?? (key === 'appearance_mode' ? 'system' : key === 'legacy_features_enabled' ? false : null) });
  }
});

test('a theme save rejects an agent update made before its commit', async ({ page, session }) => {
  const keys = ['appearance_mode', 'accent_color', 'custom_theme', 'custom_themes'];
  const before = Object.fromEntries(await Promise.all(keys.map(async key => [key, await session.invoke('get_setting', { key })])));
  let release;
  let captured;
  const held = new Promise(resolve => { release = resolve; });
  const read = new Promise(resolve => { captured = resolve; });
  const agentTheme = { id: 'test-agent-theme', name: 'Agent theme', accent: null, palette: { background: '#101010', foreground: '#FFFFFF', sidebar: null, surface: null } };
  try {
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'save_model_settings_if_unchanged') return route.continue();
      captured();
      await held;
      await route.continue();
    });
    await page.evaluate(async () => {
      const editor = await import('/src/lib/themeEditor.svelte.ts');
      editor.openThemeEditor({ seed: { background: '#FFFFFF', foreground: '#101010', sidebar: null, surface: null }, name: 'User theme' });
      void editor.saveThemeEditor();
    });
    await read;
    await session.invoke('save_setting', { key: 'custom_themes', value: [agentTheme] });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['custom_themes']));
    release();
    await expect.poll(() => page.evaluate(async () => (await import('/src/lib/themeEditor.svelte.ts')).themeEditor.error)).not.toBe('');
    expect(await session.invoke('get_setting', { key: 'custom_themes' })).toEqual([agentTheme]);
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    for (const key of keys) await session.invoke('save_setting', { key, value: before[key] ?? (key === 'appearance_mode' ? 'system' : key === 'custom_themes' ? [] : null) });
  }
});

test('a late appearance response cannot replace a newer agent change', async ({ page, session }) => {
  const previous = await session.invoke('get_setting', { key: 'appearance_mode' });
  let release;
  let captured;
  const held = new Promise(resolve => { release = resolve; });
  const read = new Promise(resolve => { captured = resolve; });
  let first = true;
  try {
    await session.invoke('save_setting', { key: 'appearance_mode', value: 'light' });
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (!first || request.command !== 'get_setting' || request.args?.key !== 'appearance_mode') return route.continue();
      first = false;
      const response = await route.fetch();
      captured();
      await held;
      await route.fulfill({ response });
    });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['appearance_mode']));
    await read;
    await session.invoke('save_setting', { key: 'appearance_mode', value: 'dark' });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['appearance_mode']));
    await expect.poll(() => page.evaluate(async () => (await import('/src/lib/stores.svelte.ts')).appStore.appearanceMode)).toBe('dark');
    const completed = page.waitForResponse(response => response.url().includes('/__verenu_dev/invoke') && response.request().postDataJSON()?.command === 'get_setting' && response.request().postDataJSON()?.args?.key === 'appearance_mode');
    release();
    await completed;
    await expect.poll(() => page.evaluate(async () => (await import('/src/lib/stores.svelte.ts')).appStore.appearanceMode)).toBe('dark');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('save_setting', { key: 'appearance_mode', value: previous ?? 'system' });
  }
});

test('a pending cleanup toggle reconciles after an agent change', async ({ page, session }) => {
  const previous = await session.invoke('get_setting', { key: 'cleanup_enabled' });
  let release;
  let captured;
  const held = new Promise(resolve => { release = resolve; });
  const read = new Promise(resolve => { captured = resolve; });
  try {
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: true });
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    const toggle = page.locator('[data-setting-target="general-cleanup"] [role="switch"]');
    await expect(toggle).toHaveAttribute('aria-checked', 'true');
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command !== 'save_setting_if_unchanged' || request.args?.key !== 'cleanup_enabled') return route.continue();
      captured();
      await held;
      await route.continue();
    });
    await toggle.click();
    await page.getByRole('button', { name: 'Turn off' }).click();
    await read;
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: true });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['cleanup_enabled']));
    await expect(toggle).toHaveAttribute('aria-checked', 'true');
    release();
    await expect.poll(() => session.invoke('get_setting', { key: 'cleanup_enabled' })).toBe(false);
    await expect(toggle).toHaveAttribute('aria-checked', 'false');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: previous ?? true });
  }
});

test('the pill keeps a newer appearance event while its startup read is delayed', async ({ page, session }) => {
  const previous = await session.invoke('get_setting', { key: 'appearance_mode' });
  let release;
  let captured;
  const held = new Promise(resolve => { release = resolve; });
  const read = new Promise(resolve => { captured = resolve; });
  try {
    await session.invoke('save_setting', { key: 'appearance_mode', value: 'light' });
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command !== 'get_setting' || request.args?.key !== 'appearance_mode') return route.continue();
      const response = await route.fetch();
      captured();
      await held;
      await route.fulfill({ response });
    });
    await page.goto(new URL('/pill.html', session.access.localAccessUrl).toString());
    await read;
    await session.invoke('save_setting', { key: 'appearance_mode', value: 'dark' });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:appearance-mode-changed', 'dark'));
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    const response = page.waitForResponse(result => result.request().postDataJSON()?.args?.key === 'appearance_mode');
    release();
    await response;
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('save_setting', { key: 'appearance_mode', value: previous ?? 'system' });
  }
});
