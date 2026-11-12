import { test, expect } from './fixtures.mjs';

const provider = {
  id: 'custom:12345678-1234-1234-8234-1234567890f1', name: 'Flow test endpoint',
  protocol: 'openai', base_url: 'http://127.0.0.1:1/v1', requires_key: false,
  supports_transcription: true, supports_cleanup: true, auth_header: null,
  extra_headers: {}, body_overrides: null,
  transcription_models: ['flow-primary', 'flow-secondary'], cleanup_models: ['flow-cleanup'],
};
const keys = ['custom_providers', 'transcription_default_model', 'cleanup_default_model', 'transcription_fallback_models', 'cleanup_fallback_models', 'dual_transcription_enabled', 'cleanup_enabled', 'transcription_provider', 'cleanup_provider', 'transcription_model', 'cleanup_model', 'transcription_models_by_provider', 'cleanup_models_by_provider', 'model_selection_mode', 'advanced_model_ui'];
const defaults = { custom_providers: [], transcription_fallback_models: [], cleanup_fallback_models: [], dual_transcription_enabled: false, cleanup_enabled: true, transcription_provider: 'groq', cleanup_provider: 'groq', transcription_models_by_provider: {}, cleanup_models_by_provider: {}, model_selection_mode: 'manual', advanced_model_ui: false };

test('Models shows the two steps in order and a setup updates them in place', async ({ page, session }, testInfo) => {
  const before = await session.invoke('get_all_settings');
  try {
    await session.invoke('save_setting', { key: 'custom_providers', value: [provider] });
    await session.invoke('save_setting', { key: 'advanced_model_ui', value: false });
    await page.reload();
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.models"]').click();

    const transcription = page.locator('[data-setting-target="models-transcription"]');
    const cleanup = page.locator('[data-setting-target="models-cleanup"]');
    await expect(transcription.locator('.head-title')).toHaveText('Transcription');
    await expect(cleanup.locator('.head-title')).toHaveText('Clean-up');
    const [first, second] = [await transcription.boundingBox(), await cleanup.boundingBox()];
    const phone = testInfo.project.name === 'phone';
    // Side by side on desktop, stacked on a phone; transcription always comes first.
    if (phone) expect(second.y).toBeGreaterThan(first.y + first.height - 1);
    else { expect(second.x).toBeGreaterThan(first.x + first.width); expect(Math.abs(second.y - first.y)).toBeLessThan(2); }

    // The switch that unlocks model choice sits with the tiles it changes, above the setups.
    const advanced = page.locator('[data-setting-target="advanced-models"]');
    const cloud = page.getByRole('region', { name: 'Cloud models' });
    expect((await advanced.boundingBox()).y).toBeGreaterThan(second.y);
    expect((await cloud.boundingBox()).y).toBeGreaterThan((await advanced.boundingBox()).y);
    await expect(transcription.getByRole('button', { name: 'Change model' })).toHaveCount(0);

    // Setups in one group share a row on desktop.
    const cards = cloud.locator('.preset-face');
    await expect(cards).toHaveCount(3);
    const boxes = await Promise.all((await cards.all()).map(card => card.boundingBox()));
    if (!phone) expect(new Set(boxes.map(box => Math.round(box.y))).size).toBe(1);

    await page.getByRole('button', { name: 'Use Quality', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Use Quality', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await expect(page.locator('.active-state')).toHaveText('Setup: Cloud Quality');
    const saved = await expect.poll(async () => (await session.invoke('get_all_settings')).model_selection_mode).toBe('quality')
      .then(() => session.invoke('get_all_settings'));
    const model = saved.transcription_default_model.split('/').slice(1).join('/');
    await expect(transcription.locator('.model-chip')).toHaveText(model);
    await expect(transcription.locator('.active-row')).toHaveCount(1);

    // Details open under the row of cards, not inside one card.
    await cloud.getByRole('button', { name: 'Details', exact: true }).first().click();
    const details = cloud.locator('.preset-details');
    await expect(details).toBeVisible();
    const detailsBox = await details.boundingBox();
    expect(detailsBox.y).toBeGreaterThan(boxes[0].y + boxes[0].height - 1);
    if (!phone) expect(detailsBox.width).toBeGreaterThan(boxes[0].width * 2);
    else expect(detailsBox.y).toBeLessThan((await cards.nth(1).boundingBox()).y);

    await page.getByRole('switch', { name: 'Advanced models' }).click();
    await expect(transcription.getByRole('button', { name: 'Change model' })).toBeVisible();
    await expect(transcription.getByRole('button', { name: 'Add fallback' })).toBeVisible();
    await expect(cleanup.getByRole('button', { name: /Edit prompt/ })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
    await page.screenshot({ path: testInfo.outputPath('models-flow.png') });
  } finally {
    for (const key of keys) await session.invoke('save_setting', { key, value: before[key] ?? defaults[key] ?? null });
  }
});
