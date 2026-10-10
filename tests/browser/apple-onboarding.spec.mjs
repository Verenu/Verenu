import { test, expect } from './fixtures.mjs';
import path from 'node:path';

// Supplemental transport fixture recreates losing availability after Models.
// Settings still use the owned native worker; this does not prove Mac inference.
test('Finish rechecks native Apple availability before saving setup choices', async ({ page, session }) => {
  const saved = await session.invoke('get_all_settings');
  const fresh = { state: 'intelligence-disabled', available: false, message: 'Presentation fixture: disabled after Models' };
  let refreshCalls = 0;
  let releaseRefresh;
  const refreshReleased = new Promise(resolve => { releaseRefresh = resolve; });
  let startedRefresh;
  const refreshStarted = new Promise(resolve => { startedRefresh = resolve; });
  try {
    // This missing-key presentation case must not depend on synthetic provider
    // credentials used by earlier production checks. No credentials are changed.
    const keyStatus = await session.invoke('get_api_key_status');
    const missingKeys = Object.fromEntries(Object.keys(keyStatus).map(provider => [provider, provider === 'local']));
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'get_api_key_status') return route.continue();
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(missingKeys) });
    });
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 3, provider: 'groq' } });
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: 'medium' });
    await page.reload();
    await page.getByRole('button', { name: "I'll add it later", exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Speed or accuracy?' })).toBeVisible();
    await page.evaluate(async () => {
      const { appleIntelligence, refreshAppleIntelligence } = await import('/src/lib/appleIntelligence.svelte.ts');
      await refreshAppleIntelligence();
      appleIntelligence.status = { state: 'available', available: true, message: 'Cached presentation fixture' };
    });
    await page.getByRole('switch', { name: 'Clean up with Apple Intelligence', exact: true }).click();
    for (let step = 0; step < 5; step++) await page.getByRole('button', { name: 'Next', exact: true }).click();
    const finish = page.getByRole('button', { name: 'Finish setup', exact: true });
    await expect(finish).toBeVisible();
    await expect(page.locator('.done-step').getByRole('button', { name: 'Add API key', exact: true })).toBeVisible();
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'get_apple_intelligence_availability') return route.fallback();
      refreshCalls++;
      startedRefresh();
      await refreshReleased;
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(fresh) });
    });
    const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
    await page.evaluate(async () => {
      await Promise.all(document.getAnimations().filter(animation => animation.effect?.getComputedTiming().iterations !== Infinity).map(animation => animation.finished.catch(() => {})));
    });
    await page.screenshot({ path: path.join(directory, `apple-finish-before-${test.info().project.name}.png`) });
    await finish.click();
    await refreshStarted;
    await expect(page.getByRole('button', { name: 'Saving…', exact: true })).toBeDisabled();
    await expect(page.locator('.done-step').getByRole('button', { name: 'Add API key', exact: true })).toBeDisabled();
    await expect(page.getByRole('button', { name: 'Saving…', exact: true })).toBeVisible();
    releaseRefresh();
    await expect(page.getByText('Apple Intelligence cleanup is selected but is not ready on this Mac.', { exact: false }).first()).toBeVisible();
    await expect(finish).toBeEnabled();
    expect(refreshCalls).toBe(1);
    expect(await page.evaluate(async () => (await import('/src/lib/appleIntelligence.svelte.ts')).appleIntelligence.status)).toEqual(fresh);
    expect(await session.invoke('get_setting', { key: 'setup_complete' })).toBe(false);
    for (const key of ['cleanup_default_model', 'cleanup_provider', 'transcription_default_model', 'transcription_provider']) {
      expect(await session.invoke('get_setting', { key })).toEqual(saved[key]);
    }
    await page.screenshot({ path: path.join(directory, `apple-finish-after-${test.info().project.name}.png`) });
    await page.getByRole('button', { name: 'Review models', exact: true }).click();
    const selected = page.getByRole('switch', { name: 'Clean up with Apple Intelligence', exact: true });
    await expect(selected).toHaveAttribute('aria-checked', 'true');
    await selected.click();
    await expect(selected).toHaveAttribute('aria-checked', 'false');
  } finally {
    releaseRefresh();
    for (const key of ['cleanup_intensity', 'setup_complete', 'setup_progress']) {
      await session.invoke('save_setting', { key, value: saved[key] ?? (key === 'setup_complete' ? true : key === 'cleanup_intensity' ? 'medium' : null) });
    }
  }
});

test('onboarding uses native eligibility for the cleanup-only Apple option', async ({ page, session }) => {
  const saved = await session.invoke('get_all_settings');
  const availability = await session.invoke('get_apple_intelligence_availability');
  const supported = ['available', 'intelligence-disabled', 'model-not-ready'].includes(availability.state);
  const modelsStep = await page.evaluate(() => /Mac/i.test(navigator.userAgent) ? 5 : 4);
  try {
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: modelsStep, provider: 'local' } });
    await page.reload();
    await expect(page.getByRole('heading', { name: 'Speed or accuracy?' })).toBeVisible();
    const choice = page.getByRole('switch', { name: 'Clean up with Apple Intelligence', exact: true });
    if (supported) {
      await expect(choice).toBeVisible();
      if (!availability.available) await expect(choice).toBeDisabled();
    } else {
      await expect(choice).toHaveCount(0);
      await expect(page.getByText('Clean up with Apple Intelligence', { exact: true })).toHaveCount(0);
    }
    // Merely offering cleanup never changes the persisted speech selection.
    expect(await session.invoke('get_setting', { key: 'transcription_default_model' })).toEqual(saved.transcription_default_model);
    expect(await session.invoke('get_setting', { key: 'cleanup_default_model' })).toEqual(saved.cleanup_default_model);
  } finally {
    await session.invoke('save_setting', { key: 'setup_complete', value: saved.setup_complete ?? true });
    await session.invoke('save_setting', { key: 'setup_progress', value: saved.setup_progress ?? null });
  }
});

// A renderer-only transition fixture: this does not establish native Mac support.
test('presentation fixture can clear a chosen cleanup after availability checking fails', async ({ page, session }) => {
  const saved = await session.invoke('get_all_settings');
  const modelsStep = await page.evaluate(() => /Mac/i.test(navigator.userAgent) ? 5 : 4);
  try {
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: modelsStep, provider: 'local' } });
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: 'medium' });
    await page.reload();
    await expect(page.getByRole('heading', { name: 'Speed or accuracy?' })).toBeVisible();
    const downloads = page.getByRole('button', { name: /^Download / });
    const originalDownloads = await downloads.allTextContents();
    await page.evaluate(async () => {
      const { appleIntelligence, refreshAppleIntelligence } = await import('/src/lib/appleIntelligence.svelte.ts');
      await refreshAppleIntelligence();
      appleIntelligence.status = { state: 'available', available: true, message: 'Presentation fixture' };
    });
    const choice = page.getByRole('switch', { name: 'Clean up with Apple Intelligence', exact: true });
    await choice.click();
    await expect(choice).toHaveAttribute('aria-checked', 'true');
    await page.evaluate(async () => {
      const { appleIntelligence } = await import('/src/lib/appleIntelligence.svelte.ts');
      appleIntelligence.status = { state: 'unavailable', available: false, message: 'Presentation fixture: check failed' };
    });
    await expect(choice).toHaveCount(0);
    const recovery = page.getByRole('button', { name: 'Turn off selected cleanup', exact: true });
    await expect(recovery).toBeVisible();
    await recovery.click();
    await expect(recovery).toHaveCount(0);
    expect(await downloads.allTextContents()).toEqual(originalDownloads);
    expect(await session.invoke('get_setting', { key: 'cleanup_default_model' })).toEqual(saved.cleanup_default_model);
    expect(await session.invoke('get_setting', { key: 'transcription_default_model' })).toEqual(saved.transcription_default_model);
  } finally {
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: saved.cleanup_intensity ?? 'medium' });
    await session.invoke('save_setting', { key: 'setup_complete', value: saved.setup_complete ?? true });
    await session.invoke('save_setting', { key: 'setup_progress', value: saved.setup_progress ?? null });
  }
});

// Supplemental presentation regression. Native eligibility is tested above;
// this state fixture cannot establish supported-Mac behavior or inference.
test('presentation fixture keeps Apple cleanup inactive when onboarding cleanup is Off', async ({ page, session }) => {
  const saved = await session.invoke('get_all_settings');
  const modelsStep = await page.evaluate(() => /Mac/i.test(navigator.userAgent) ? 5 : 4);
  try {
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: modelsStep, provider: 'local' } });
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: 'none' });
    await page.reload();
    await expect(page.getByRole('heading', { name: 'Speed or accuracy?' })).toBeVisible();
    const downloads = page.getByRole('button', { name: /^Download / });
    const originalDownloads = await downloads.allTextContents();
    await page.evaluate(async () => {
      const { appleIntelligence, refreshAppleIntelligence } = await import('/src/lib/appleIntelligence.svelte.ts');
      await refreshAppleIntelligence();
      appleIntelligence.status = { state: 'available', available: true, message: 'Presentation fixture' };
    });
    const choice = page.getByRole('switch', { name: 'Clean up with Apple Intelligence', exact: true });
    await expect(choice).toBeVisible();
    await choice.click();
    await expect(choice).toHaveAttribute('aria-checked', 'true');
    await expect(page.getByText('Cleanup is off, so Apple Intelligence is not used.', { exact: false })).toBeVisible();
    expect(await downloads.allTextContents()).toEqual(originalDownloads);
    await page.evaluate(async () => {
      const { appleIntelligence } = await import('/src/lib/appleIntelligence.svelte.ts');
      appleIntelligence.status = { state: 'model-not-ready', available: false, message: 'Presentation fixture: model downloading' };
    });
    await expect(choice).toHaveAttribute('aria-checked', 'true');
    await expect(page.getByText('Selected, but not ready.', { exact: false })).toHaveCount(0);
    expect(await session.invoke('get_setting', { key: 'cleanup_default_model' })).toEqual(saved.cleanup_default_model);
    expect(await session.invoke('get_setting', { key: 'transcription_default_model' })).toEqual(saved.transcription_default_model);
  } finally {
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: saved.cleanup_intensity ?? 'medium' });
    await session.invoke('save_setting', { key: 'setup_complete', value: true });
    await session.invoke('save_setting', { key: 'setup_progress', value: saved.setup_progress ?? null });
  }
});
