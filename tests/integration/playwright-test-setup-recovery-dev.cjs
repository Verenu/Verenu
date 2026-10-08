'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium, expect } = require('playwright/test');
const { TARGET_URL, seedDevState, closeSettings } = require('./_dev-helpers.cjs');

const base = {
  setup_complete: true,
  force_setup_on_launch: false,
  analytics_enabled: false,
  verenu_service_checks_enabled: false,
  cleanup_enabled: false,
};
const failure = {
  id: 'dictation', requested: 'Ctrl+Super', active: null, codes: [],
  note: 'No available shortcut. Change the desktop binding or choose another shortcut.',
};
const screenshotDir = process.env.SCREENSHOT_DIR;
const screenshotNames = new Set((process.env.SCREENSHOT_NAMES || '').split(',').map(name => name.trim()).filter(Boolean));
if (screenshotDir) fs.mkdirSync(screenshotDir, { recursive: true });

(async () => {
  const browser = await chromium.launch({ headless: true });
  const errors = [];
  async function screenshot(page, name, fullPage = true) {
    if (screenshotDir && (!screenshotNames.size || screenshotNames.has(name))) {
      await page.screenshot({ path: path.join(screenshotDir, `${name}.png`), fullPage });
    }
  }
  async function open(settings = {}, localSttModels = {}, localLlmModels = {}, runtimeInstalled = false, localStates = {}) {
    const page = await browser.newPage({ viewport: { width: 900, height: 600 }, reducedMotion: 'reduce' });
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/*', route => new URL(route.request().url()).origin === new URL(TARGET_URL).origin ? route.continue() : route.abort());
    await seedDevState(page, {
      settings: { ...base, ...settings },
      localSttModels,
      localLlmModels,
      localSttState: localStates.transcription ?? null,
      localLlmState: localStates.cleanup ?? null,
    });
    await page.addInitScript(installed => localStorage.setItem('verenu:dev-local-llm-runtime', JSON.stringify({ installed, is_downloading: false })), runtimeInstalled);
    await page.goto(TARGET_URL, { waitUntil: 'networkidle' });
    await guard(page);
    return page;
  }
  async function guard(page) {
    await page.evaluate(async () => {
      const { frontendIpcActivity } = await import('/src/lib/diagnostics.ts');
      const start = frontendIpcActivity.start.bind(frontendIpcActivity);
      window.__blockedSetupCommands = [];
      frontendIpcActivity.start = command => {
        if (/^download_|validate_api_key|start_.*recording|transcribe|check_provider_status/.test(command)) {
          window.__blockedSetupCommands.push(command);
          throw new Error(`Forbidden verification command: ${command}`);
        }
        return start(command);
      };
      const { localModelDownloads } = await import('/src/lib/components/settings/localModelDownloads.ts');
      localModelDownloads.transcription.download = async () => false;
      localModelDownloads.cleanup.download = async () => false;
    });
  }
  async function publish(page, status) {
    await page.evaluate(async item => {
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('verenu:shortcuts-changed', [item]);
    }, status);
  }
  async function assertSafe(page) {
    assert.deepEqual(await page.evaluate(() => window.__blockedSetupCommands), []);
    const commands = await page.evaluate(async () => {
      const { frontendIpcActivity } = await import('/src/lib/diagnostics.ts');
      return frontendIpcActivity.snapshot().map(item => item.command);
    });
    assert(!commands.some(command => /^download_|validate_api_key|start_.*recording|transcribe|check_provider_status/.test(command)), 'No model download, provider validation, recording, or provider-status command should run');
  }

  async function completeLocalSetup({
    localSttModels = {},
    localLlmModels = {},
    localSttState = { current_model_id: null, is_loaded: false, is_loading: false, is_downloading: false, downloading_model_id: null },
    runtimeInstalled = false,
    presetName = 'Transcription only',
    chooseMissingSpeech = false,
    requireInstalledPreset = false,
    cleanupChoice = null,
  } = {}) {
    const page = await open({ setup_complete: false, force_setup_on_launch: true }, localSttModels, localLlmModels, runtimeInstalled, { transcription: localSttState });
    await page.getByRole('button', { name: 'Get Started' }).click();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await page.locator('.provider-card:has-text("On this device")').click();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await page.getByRole('button', { name: 'Continue', exact: true }).click();
    const selectedPreset = page.locator('.models-picker .preset-row').filter({ hasText: presetName });
    await expect(selectedPreset).toBeVisible();
    if (requireInstalledPreset) {
      await selectedPreset.getByRole('button', { name: `Use local ${presetName}`, exact: true }).click();
    } else if (chooseMissingSpeech || await selectedPreset.getByRole('button', { name: /^Download/ }).count()) {
      await selectedPreset.getByRole('button', { name: /^Download/ }).click();
    } else {
      await selectedPreset.getByRole('button', { name: `Use local ${presetName}`, exact: true }).click();
    }
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    if (cleanupChoice) {
      await page.locator('button.pick-card', { has: page.locator('.card-name', { hasText: cleanupChoice }) }).click();
    }
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await expect(page.locator('.setup-overlay .done-step')).toBeVisible();
    return page;
  }

  try {
    const missing = await open();
    await expect(missing.getByText('Speech recognition needs an API key for groq.')).toBeVisible();
    await expect(missing.getByRole('heading', { name: 'Finish dictation setup' })).toBeVisible();
    await screenshot(missing, 'missing-setup-home');
    await missing.getByRole('button', { name: 'Add API key', exact: true }).click();
    await expect(missing.locator('.settings-page').getByRole('heading', { name: 'Providers', exact: true })).toBeVisible();
    await expect(missing.getByRole('button', { name: 'Back to setup', exact: true })).toHaveCount(0);
    await closeSettings(missing);
    await missing.close();

    const readyCloud = await open({
      cleanup_enabled: true,
      transcription_default_model: 'groq/whisper-large-v3-turbo',
      cleanup_default_model: 'openai/gpt-4o-mini',
      __provider_connected: { groq: true, openai: true },
    });
    await expect(readyCloud.getByRole('heading', { name: /Hold Ctrl.*Super.*to dictate/ })).toBeVisible();
    await expect(readyCloud.locator('.readiness-notice, .shortcut-recovery, .error-toast')).toHaveCount(0);
    await screenshot(readyCloud, 'ready-cloud');
    await assertSafe(readyCloud);
    await readyCloud.close();

    const offlineFusion = await open({
      cleanup_enabled: true, cleanup_intensity: 'none', dual_transcription_enabled: true,
      transcription_default_model: 'local/parakeet-v3',
      transcription_fallback_models: ['groq/whisper-large-v3-turbo'],
      cleanup_default_model: 'local/readiness-missing-cleanup', cleanup_fallback_models: [],
      __provider_connected: { groq: true },
    }, { 'parakeet-v3': { downloaded: true } });
    await expect(offlineFusion.locator('.readiness-notice')).toContainText('readiness-missing-cleanup');
    await offlineFusion.evaluate(async () => {
      const { appStore } = await import('/src/lib/stores.ts'); appStore.isOnline = false;
    });
    await expect(offlineFusion.locator('.readiness-notice')).toHaveCount(0);
    await offlineFusion.setViewportSize({ width: 390, height: 844 });
    await expect(offlineFusion.locator('.readiness-notice')).toHaveCount(0);
    await offlineFusion.evaluate(async () => {
      const { appStore } = await import('/src/lib/stores.ts'); appStore.isOnline = true;
    });
    await expect(offlineFusion.locator('.readiness-notice')).toContainText('readiness-missing-cleanup');
    await assertSafe(offlineFusion);
    await offlineFusion.close();

    const readyLocal = await open({
      cleanup_enabled: false,
      cleanup_intensity: 'none',
      transcription_default_model: 'local/parakeet-v3',
    }, { 'parakeet-v3': { downloaded: true } });
    await expect(readyLocal.getByRole('heading', { name: /Hold Ctrl.*Super.*to dictate/ })).toBeVisible();
    await expect(readyLocal.locator('.readiness-notice, .shortcut-recovery, .error-toast')).toHaveCount(0);
    await screenshot(readyLocal, 'ready-local-speech-only');
    await assertSafe(readyLocal);
    await readyLocal.close();

    const fallback = await open({
      cleanup_enabled: true,
      cleanup_intensity: 'medium',
      transcription_default_model: 'groq/whisper-large-v3-turbo',
      cleanup_default_model: 'openai/gpt-4o-mini',
      transcription_fallback_models: ['local/parakeet-v3'],
      cleanup_fallback_models: ['local/qwen2.5-3b-instruct'],
    }, { 'parakeet-v3': { downloaded: true } }, { 'qwen2.5-3b-instruct': { downloaded: true } }, true);
    await expect(fallback.locator('.readiness-notice')).toHaveCount(0);
    await assertSafe(fallback);
    await fallback.close();

    const recovery = await open({
      __provider_connected: { groq: true },
      transcription_default_model: 'groq/whisper-large-v3-turbo',
    });
    await publish(recovery, failure);
    await expect(recovery.getByRole('heading', { name: 'Dictation shortcut unavailable' })).toBeVisible();
    await expect(recovery.getByText(/Xfce\/X11 shortcut capture is not supported/)).toBeVisible();
    await screenshot(recovery, 'shortcut-recovery-home');
    await recovery.getByRole('button', { name: 'Open shortcut settings' }).click();
    await expect(recovery.locator('.settings-page').getByRole('heading', { name: 'General', exact: true })).toBeVisible();
    await closeSettings(recovery);
    await expect(recovery.getByRole('heading', { name: 'Dictation shortcut unavailable' })).toBeVisible();
    await assertSafe(recovery);
    await publish(recovery, { ...failure, active: 'Ctrl+Super', codes: ['ControlLeft', 'MetaLeft'], note: null });
    await expect(recovery.locator('.shortcut-recovery')).toHaveCount(0);
    await recovery.close();

    const setup = await open({ setup_complete: false, force_setup_on_launch: true }, { 'parakeet-v3': { downloaded: true } });
    await publish(setup, failure);
    await expect(setup.locator('.setup-overlay .shortcut-recovery')).toBeVisible();
    await screenshot(setup, 'setup-intro-recovery');
    await setup.locator('.setup-overlay').getByRole('button', { name: 'Open shortcut settings' }).click();
    await expect(setup.locator('.settings-page').getByRole('heading', { name: 'General', exact: true })).toBeVisible();
    await expect(setup.getByRole('button', { name: 'Back to setup', exact: true })).toBeVisible();
    await screenshot(setup, 'settings-return-to-setup');
    await setup.getByRole('button', { name: 'Back to setup', exact: true }).click();
    await expect(setup.locator('.settings-overlay-wrap')).toHaveCount(0);
    await expect(setup.getByRole('button', { name: 'Get Started' })).toBeVisible();
    await setup.getByRole('button', { name: 'Get Started' }).click();
    await expect(setup.getByRole('heading', { name: 'Choose whether to share product analytics' })).toBeVisible();
    await setup.getByRole('button', { name: 'Next', exact: true }).click();
    await expect(setup.getByRole('heading', { name: 'Choose your AI provider' })).toBeVisible();
    await setup.locator('.provider-card:has-text("On this device")').click();
    await setup.getByRole('button', { name: 'Next', exact: true }).click();
    await setup.getByRole('button', { name: 'Continue', exact: true }).click();
    await expect(setup.getByRole('button', { name: 'Use local Transcription only', exact: true })).toBeVisible();
    await setup.getByRole('button', { name: 'Use local Transcription only', exact: true }).click();
    await expect(setup.getByRole('button', { name: 'Use local Transcription only', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await screenshot(setup, 'transcription-only-choice');
    const overflows = await setup.evaluate(() => document.querySelector('.models-picker').getBoundingClientRect().bottom > document.querySelector('.setup-actionbar').getBoundingClientRect().top);
    assert.equal(overflows, false, 'Transcription-only preset fits above the setup action bar at 900x600');
    await setup.getByRole('button', { name: 'Next', exact: true }).click();
    await setup.getByRole('button', { name: 'Next', exact: true }).click();
    await setup.getByRole('button', { name: 'Next', exact: true }).click();
    await setup.getByRole('button', { name: 'Next', exact: true }).click();
    await expect(setup.getByRole('heading', { name: 'Give it a try' })).toBeVisible();
    await publish(setup, failure);
    await expect(setup.locator('.setup-overlay .shortcut-recovery')).toBeVisible();
    await setup.waitForTimeout(400);
    await screenshot(setup, 'try-it-shortcut-recovery');
    await setup.locator('.setup-overlay').getByRole('button', { name: 'Open shortcut settings' }).click();
    await expect(setup.locator('.settings-page').getByRole('heading', { name: 'General', exact: true })).toBeVisible();
    await setup.getByRole('button', { name: 'Back to setup', exact: true }).click();
    await expect(setup.locator('.settings-overlay-wrap')).toHaveCount(0);
    await expect(setup.getByRole('heading', { name: 'Give it a try' })).toBeVisible();
    await publish(setup, { ...failure, active: 'Ctrl+Super', codes: ['ControlLeft', 'MetaLeft'], note: null });
    await expect(setup.locator('.setup-overlay .shortcut-recovery')).toHaveCount(0);
    await assertSafe(setup);
    await setup.close();

    const setupLocalWithCloudKey = await open({
      setup_complete: false,
      force_setup_on_launch: true,
      __provider_connected: { groq: true },
    }, { 'parakeet-v3': { downloaded: true } }, { 'qwen2.5-3b-instruct': { downloaded: true } }, true);
    await expect(setupLocalWithCloudKey.getByRole('button', { name: 'Get Started' })).toBeVisible();
    await setupLocalWithCloudKey.getByRole('button', { name: 'Get Started' }).click();
    await setupLocalWithCloudKey.getByRole('button', { name: 'Next', exact: true }).click();
    await setupLocalWithCloudKey.locator('.provider-card:has-text("On this device")').click();
    await setupLocalWithCloudKey.getByRole('button', { name: 'Next', exact: true }).click();
    await setupLocalWithCloudKey.getByRole('button', { name: 'Continue', exact: true }).click();
    await expect(setupLocalWithCloudKey.getByRole('button', { name: 'Use local Balanced', exact: true }))
      .toHaveAttribute('aria-pressed', 'true');
    await expect(setupLocalWithCloudKey.getByRole('button', { name: 'Next', exact: true })).toBeEnabled();
    await screenshot(setupLocalWithCloudKey, 'local-balanced-preselection');
    await assertSafe(setupLocalWithCloudKey);
    await setupLocalWithCloudKey.close();

    const unsupportedLocalResume = await open({
      setup_complete: false,
      force_setup_on_launch: true,
      transcription_provider: 'local',
      setup_progress: { step: 4, provider: 'local' },
      __local_models_supported: false,
    });
    const unsupportedRecovery = unsupportedLocalResume.locator('.local-support-recovery[data-support="unsupported"]');
    await expect(unsupportedRecovery).toBeVisible();
    await expect(unsupportedRecovery).toContainText('Intel Macs');
    await expect(unsupportedLocalResume.getByRole('button', { name: 'Open API keys' })).toHaveCount(0);
    await expect(unsupportedLocalResume.getByRole('button', { name: 'Choose a setup', exact: true })).toBeDisabled();
    await screenshot(unsupportedLocalResume, 'setup-local-unsupported-desktop', false);
    await unsupportedLocalResume.setViewportSize({ width: 900, height: 500 });
    await screenshot(unsupportedLocalResume, 'setup-local-unsupported-short', false);
    await unsupportedRecovery.getByRole('button', { name: 'Choose a cloud provider' }).click();
    await expect(unsupportedLocalResume.getByRole('heading', { name: 'Choose your AI provider' })).toBeVisible();
    await expect(unsupportedLocalResume.locator('.provider-card:has-text("On this device")')).toHaveCount(0);
    await expect(unsupportedLocalResume.locator('.provider-step .availability-note')).toContainText('Intel Macs');
    await expect(unsupportedLocalResume.locator('.provider-card:has-text("Groq")')).toHaveAttribute('aria-pressed', 'true');
    await unsupportedLocalResume.getByRole('button', { name: 'Next', exact: true }).click();
    await expect(unsupportedLocalResume.getByText('Do you already have a Groq API key?')).toBeVisible();
    await unsupportedLocalResume.getByRole('button', { name: "I'll add it later", exact: true }).click();
    await expect(unsupportedLocalResume.locator('.models-picker .preset-row')).toContainText('Add an API key');
    await unsupportedLocalResume.getByRole('button', { name: 'Open API keys', exact: true }).click();
    await expect(unsupportedLocalResume.getByText('Do you already have a Groq API key?')).toBeVisible();
    await expect(unsupportedLocalResume.locator('.local-setup')).toHaveCount(0);
    await assertSafe(unsupportedLocalResume);
    await unsupportedLocalResume.close();

    const unknownLocalResume = await open({
      setup_complete: false,
      force_setup_on_launch: true,
      transcription_provider: 'local',
      setup_progress: { step: 4, provider: 'local' },
      __local_models_supported: 'error',
    });
    const unknownRecovery = unknownLocalResume.locator('.local-support-recovery[data-support="unknown"]');
    await expect(unknownRecovery).toBeVisible();
    await expect(unknownRecovery).toContainText('Could not confirm');
    await expect(unknownRecovery).not.toContainText('Intel');
    await expect(unknownLocalResume.getByRole('button', { name: 'Open API keys' })).toHaveCount(0);
    await unknownLocalResume.evaluate(() => {
      const settings = JSON.parse(localStorage.getItem('verenu:dev-settings') || '{}');
      localStorage.setItem('verenu:dev-settings', JSON.stringify({ ...settings, __local_models_supported: true }));
    });
    await unknownRecovery.getByRole('button', { name: 'Retry check' }).click();
    await expect(unknownLocalResume.locator('.models-picker .preset-row').filter({ hasText: 'Transcription only' })).toBeVisible();
    await assertSafe(unknownLocalResume);
    await unknownLocalResume.close();

    const unknownCloudChoice = await open({
      setup_complete: false,
      force_setup_on_launch: true,
      transcription_provider: 'local',
      setup_progress: { step: 4, provider: 'local' },
      __local_models_supported: 'error',
    });
    await expect(unknownCloudChoice.locator('.local-support-recovery[data-support="unknown"]')).toBeVisible();
    await unknownCloudChoice.getByRole('button', { name: 'Choose a cloud provider' }).click();
    await expect(unknownCloudChoice.getByRole('heading', { name: 'Choose your AI provider' })).toBeVisible();
    await expect(unknownCloudChoice.locator('.provider-card:has-text("On this device")')).toHaveCount(0);
    await expect(unknownCloudChoice.locator('.provider-step .availability-note')).toContainText('Could not confirm');
    await expect(unknownCloudChoice.locator('.provider-step .availability-note')).not.toContainText('Intel');
    await assertSafe(unknownCloudChoice);
    await unknownCloudChoice.close();

    const pendingLocalDone = await completeLocalSetup({
      localSttModels: { 'parakeet-v3': { downloaded: false, partial_size: 512 } },
      localSttState: {
        current_model_id: null, is_loaded: false, is_loading: false,
        is_downloading: true, downloading_model_id: 'parakeet-v3',
      },
      chooseMissingSpeech: true,
    });
    await expect(pendingLocalDone.getByRole('heading', { name: 'Your choices are saved when you finish.' })).toBeVisible();
    await expect(pendingLocalDone.locator('.done-model-warning')).toContainText('speech model is still downloading');
    await expect(pendingLocalDone.locator('.done-model-warning')).toContainText('Open Models settings');
    await expect(pendingLocalDone.getByRole('button', { name: 'Finish setup', exact: true })).toBeEnabled();
    await expect(pendingLocalDone.locator('.done-warning').filter({ hasText: 'No API key set' })).toHaveCount(0);
    await screenshot(pendingLocalDone, 'setup-done-local-pending');
    await pendingLocalDone.setViewportSize({ width: 390, height: 844 });
    await screenshot(pendingLocalDone, 'setup-done-local-pending-phone', false);
    await pendingLocalDone.setViewportSize({ width: 900, height: 600 });
    await assertSafe(pendingLocalDone);

    await pendingLocalDone.getByRole('button', { name: 'Open Models settings', exact: true }).click();
    await expect(pendingLocalDone.locator('.settings-page')).toBeVisible();
    await expect(pendingLocalDone.getByRole('button', { name: 'Back to setup', exact: true })).toBeVisible();
    await pendingLocalDone.getByRole('button', { name: 'Back to setup', exact: true }).click();
    await expect(pendingLocalDone.locator('.settings-page')).toHaveCount(0);
    await expect(pendingLocalDone.locator('.done-model-warning')).toContainText('speech model is still downloading');

    await pendingLocalDone.evaluate(async () => {
      const { invoke } = await import('/src/lib/tauri.ts');
      await invoke('cancel_local_stt_model_download', { modelId: 'parakeet-v3' });
    });
    await expect(pendingLocalDone.locator('.done-model-warning')).toContainText("speech model isn't installed yet");
    await assertSafe(pendingLocalDone);

    await pendingLocalDone.evaluate(async () => {
      localStorage.setItem('verenu:dev-local-stt-models', JSON.stringify({ 'parakeet-v3': { downloaded: true, partial_size: 0 } }));
      localStorage.setItem('verenu:dev-local-stt-state', JSON.stringify({
        current_model_id: null, is_loaded: false, is_loading: false,
        is_downloading: false, downloading_model_id: null,
      }));
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('local-stt-model-download-complete', { model_id: 'parakeet-v3', error: null });
    });
    await expect(pendingLocalDone.locator('.done-model-warning')).toHaveCount(0);
    await expect(pendingLocalDone.getByRole('heading', { name: "You're all set." })).toBeVisible();
    await screenshot(pendingLocalDone, 'setup-done-local-installed');
    await pendingLocalDone.setViewportSize({ width: 390, height: 844 });
    await screenshot(pendingLocalDone, 'setup-done-local-installed-phone', false);
    await pendingLocalDone.setViewportSize({ width: 900, height: 600 });
    await assertSafe(pendingLocalDone);
    await pendingLocalDone.close();

    const missingLocalDone = await completeLocalSetup({
      localSttModels: { 'parakeet-v3': { downloaded: false, partial_size: 0 } },
      chooseMissingSpeech: true,
    });
    await expect(missingLocalDone.locator('.done-model-warning')).toContainText("speech model isn't installed yet");
    await expect(missingLocalDone.locator('.done-model-warning')).toContainText('Open Models settings');
    await expect(missingLocalDone.locator('.done-warning').filter({ hasText: 'No API key set' })).toHaveCount(0);
    await assertSafe(missingLocalDone);
    await missingLocalDone.close();

    const installedLocalDone = await completeLocalSetup({
      localSttModels: { 'parakeet-v3': { downloaded: true, partial_size: 0 } },
    });
    await expect(installedLocalDone.locator('.done-model-warning')).toHaveCount(0);
    await expect(installedLocalDone.getByRole('heading', { name: "You're all set." })).toBeVisible();
    await expect(installedLocalDone.locator('.done-warning').filter({ hasText: 'No API key set' })).toHaveCount(0);
    await assertSafe(installedLocalDone);
    await installedLocalDone.close();

    const cleanupOffDone = await completeLocalSetup({
      presetName: 'Balanced',
      localSttModels: { 'parakeet-v3': { downloaded: true, partial_size: 0 } },
      cleanupChoice: 'Off',
    });
    await expect(cleanupOffDone.getByRole('heading', { name: "You're all set." })).toBeVisible();
    await expect(cleanupOffDone.locator('.done-model-warning')).toHaveCount(0);
    await expect(cleanupOffDone.locator('.done-warning').filter({ hasText: 'No API key set' })).toHaveCount(0);
    await screenshot(cleanupOffDone, 'setup-done-local-cleanup-off-desktop', false);
    await cleanupOffDone.setViewportSize({ width: 900, height: 500 });
    await screenshot(cleanupOffDone, 'setup-done-local-cleanup-off-short', false);
    await assertSafe(cleanupOffDone);
    await cleanupOffDone.close();

    const installedLocalCleanupDone = await completeLocalSetup({
      presetName: 'Balanced',
      localSttModels: { 'parakeet-v3': { downloaded: true, partial_size: 0 } },
      localLlmModels: { 'qwen2.5-3b-instruct': { downloaded: true, partial_size: 0 } },
      runtimeInstalled: true,
      requireInstalledPreset: true,
      cleanupChoice: 'Medium',
    });
    await installedLocalCleanupDone.locator('.tryit-callout').waitFor({ state: 'hidden' });
    await installedLocalCleanupDone.waitForFunction(() => document.getAnimations().every(animation =>
      animation.playState !== 'running' || animation.effect?.getComputedTiming().iterations === Infinity));
    await screenshot(installedLocalCleanupDone, 'installed-cleanup-desktop');
    await installedLocalCleanupDone.setViewportSize({ width: 390, height: 844 });
    await screenshot(installedLocalCleanupDone, 'installed-cleanup-phone');
    await expect(installedLocalCleanupDone.locator('.done-model-warning')).toHaveCount(0);
    await expect(installedLocalCleanupDone.getByRole('heading', { name: "You're all set." })).toBeVisible();
    await assertSafe(installedLocalCleanupDone);
    await installedLocalCleanupDone.close();

    const cleanupOnDone = await completeLocalSetup({
      presetName: 'Balanced',
      localSttModels: { 'parakeet-v3': { downloaded: true, partial_size: 0 } },
      cleanupChoice: 'Medium',
    });
    await expect(cleanupOnDone.locator('.done-model-warning')).toContainText('cleanup model');
    await expect(cleanupOnDone.locator('.done-model-warning')).toContainText('cleanup engine');
    await expect(cleanupOnDone.locator('.done-model-warning')).not.toContainText('speech model');
    await assertSafe(cleanupOnDone);
    await cleanupOnDone.close();

    const missingCleanupLocalDone = await completeLocalSetup({
      presetName: 'Fastest',
      chooseMissingSpeech: true,
    });
    await expect(missingCleanupLocalDone.locator('.done-model-warning')).toContainText('cleanup model');
    await expect(missingCleanupLocalDone.locator('.done-model-warning')).toContainText('cleanup engine');
    await expect(missingCleanupLocalDone.locator('.done-warning').filter({ hasText: 'No API key set' })).toHaveCount(0);
    await assertSafe(missingCleanupLocalDone);
    await missingCleanupLocalDone.close();

    assert.deepEqual(errors, []);
    console.log('PASS - home readiness, shortcut recovery, wizard resume, local and cloud preset selection, unsupported and unknown local-platform recovery, and Done model readiness with cleanup Off and enabled; no model-download, provider, or recording IPC');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
