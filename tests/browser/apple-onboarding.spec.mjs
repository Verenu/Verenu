import { test, expect } from './fixtures.mjs';

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
