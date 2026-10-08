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
if (screenshotDir) fs.mkdirSync(screenshotDir, { recursive: true });

(async () => {
  const browser = await chromium.launch({ headless: true });
  const errors = [];
  async function screenshot(page, name) {
    if (screenshotDir) await page.screenshot({ path: path.join(screenshotDir, `${name}.png`), fullPage: true });
  }
  async function open(settings = {}, localSttModels = {}, localLlmModels = {}, runtimeInstalled = false) {
    const page = await browser.newPage({ viewport: { width: 900, height: 600 }, reducedMotion: 'reduce' });
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/*', route => new URL(route.request().url()).origin === new URL(TARGET_URL).origin ? route.continue() : route.abort());
    await seedDevState(page, { settings: { ...base, ...settings }, localSttModels, localLlmModels });
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

    assert.deepEqual(errors, []);
    console.log('PASS - home readiness, configured fallback readiness, shortcut recovery, wizard state preservation, and local transcription-only choice; no downloads or provider calls');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
