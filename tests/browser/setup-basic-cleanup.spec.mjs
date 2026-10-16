import { test, expect } from './fixtures.mjs';
import path from 'node:path';

async function captureSetupState(page, name) {
  await page.evaluate(async () => {
    await Promise.all(document.getAnimations()
      .filter(animation => animation.effect?.getComputedTiming().iterations !== Infinity)
      .map(animation => animation.finished.catch(() => {})));
  });
  await page.screenshot({ path: path.join(path.dirname(process.env.VERENU_SESSION_ACCESS_FILE), name) });
}

async function installAppleAvailabilityProbe(page) {
  await page.addInitScript(() => {
    const realFetch = window.fetch;
    const calls = [];
    const probe = { calls, installed: false };
    window.__appleAvailabilityProbe = probe;
    window.fetch = function (input, init) {
      let isAvailabilityCheck = false;
      try {
        const url = typeof input === 'string' ? input : input?.url ?? '';
        isAvailabilityCheck = url.endsWith('/__verenu_dev/invoke') && typeof init?.body === 'string'
          && JSON.parse(init.body).command === 'get_apple_intelligence_availability';
      } catch {   }
      if (isAvailabilityCheck) {
        const limit = Error.stackTraceLimit;
        Error.stackTraceLimit = 100;
        const stack = new Error().stack ?? '';
        Error.stackTraceLimit = limit;
        calls.push([...new Set(stack.match(/\/src\/[^\s:?)]+/g) ?? [])]);
      }
      return realFetch.apply(this, arguments);
    };
    probe.installed = window.fetch !== realFetch;
  });
}

async function appleAvailabilityOrigins(page) {
  const probe = await page.evaluate(() => ({
    installed: window.__appleAvailabilityProbe?.installed === true,
    calls: window.__appleAvailabilityProbe?.calls ?? null,
  }));
  expect(probe.installed, 'Apple availability probe is installed').toBe(true);
  return probe.calls.map(modules => {
    if (modules.some(file => file === '/src/lib/views/Setup.svelte' || file.startsWith('/src/lib/setup/'))) return 'setup';
    if (modules.includes('/src/lib/views/home/DictationReadiness.svelte')) return 'readiness-background';
    return 'unknown';
  });
}

const defaults = {
  setup_complete: true, setup_progress: null, cleanup_intensity: 'medium', default_tone: 'casual',
  cleanup_enabled: true, transcription_provider: 'groq', cleanup_provider: 'groq',
  transcription_models_by_provider: {}, cleanup_models_by_provider: {},
  transcription_fallback_models: [], cleanup_fallback_models: [],
  dual_transcription_enabled: false, transcription_language: 'en', appearance_mode: 'system', mute_audio: false,
};

const SETUP_KEYS = [
  'setup_progress', 'setup_complete', 'cleanup_intensity', 'default_tone', 'cleanup_enabled',
  'transcription_provider', 'transcription_model', 'transcription_default_model',
  'transcription_models_by_provider', 'transcription_fallback_models', 'dual_transcription_enabled',
  'transcription_language', 'cleanup_provider', 'cleanup_model', 'cleanup_default_model',
  'cleanup_models_by_provider', 'cleanup_fallback_models', 'appearance_mode', 'mute_audio',
];

async function snapshotSetup(session) {
  const previous = await session.invoke('get_all_settings');
  for (const key of ['cleanup_intensity', 'default_tone', 'setup_complete', 'setup_progress']) {
    previous[key] = await session.invoke('get_setting', { key });
  }
  return previous;
}

async function restoreSetup(session, previous) {
  for (const key of SETUP_KEYS) await session.invoke('save_setting', { key, value: previous[key] ?? defaults[key] ?? null });
}

for (const priorAppleChoice of [false, true]) {
test(`Setup keeps Basic cleanup on after a transcription-only preset${priorAppleChoice ? ' and an unavailable Apple choice' : ''}`, async ({ page, session }, testInfo) => {
  const previous = await snapshotSetup(session);
  try {
    await installAppleAvailabilityProbe(page);
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
    if (priorAppleChoice) {
      await page.evaluate(async () => {
        const { appleIntelligence, refreshAppleIntelligence } = await import('/src/lib/appleIntelligence.svelte.ts');
        await refreshAppleIntelligence();
        appleIntelligence.status = { state: 'available', available: true, message: 'Synthetic setup fixture' };
      });
      await page.getByRole('switch', { name: 'Clean up with Apple Intelligence', exact: true }).click();
    }
    await page.getByRole('button', { name: 'Use local Transcription only' }).click();
    await expect(page.getByRole('button', { name: 'Use local Transcription only' })).toHaveAttribute('aria-pressed', 'true');
    await page.getByRole('button', { name: 'Next', exact: true }).click();

    await page.getByRole('button', { name: /^Basic / }).click();
    await expect(page.getByRole('button', { name: /^Basic / })).toHaveAttribute('aria-pressed', 'true');
    if (priorAppleChoice) {
      await page.evaluate(async () => {
        (await import('/src/lib/appleIntelligence.svelte.ts')).appleIntelligence.status = {
          state: 'intelligence-disabled', available: false, message: 'Synthetic fixture: unavailable after Basic choice',
        };
      });
      await page.route('**/__verenu_dev/invoke', async route => {
        if (route.request().postDataJSON()?.command !== 'get_apple_intelligence_availability') return route.continue();
        await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({
          state: 'intelligence-disabled', available: false, message: 'Synthetic fixture: unavailable',
        }) });
      });
    }

    if (priorAppleChoice) {
      await page.getByRole('button', { name: 'Step 4', exact: true }).click();
      await page.evaluate(async () => {
        await (await import('/src/lib/appleIntelligence.svelte.ts')).refreshAppleIntelligence();
        const { localSttStore } = await import('/src/lib/localSttStore.svelte.ts');
        const { invoke } = await import('/src/lib/tauri.ts');
        localSttStore.models = (await invoke('list_local_stt_models')).map(model => ({ ...model, is_downloaded: true, is_downloading: false }));
      });
      await expect(page.getByText('Basic cleanup runs on this device, so Apple Intelligence is not used.', { exact: false })).toBeVisible();
      await expect(page.getByText('Setup cannot finish', { exact: false })).toHaveCount(0);
      await captureSetupState(page, `basic-apple-models-${testInfo.project.name}.png`);
    }
    for (let i = 0; i < 6 && !(await page.getByText('Finish setup', { exact: true }).isVisible()); i++) {
      await page.getByRole('button', { name: 'Next', exact: true }).click();
    }
    await expect(page.getByText('Finish setup', { exact: true })).toBeVisible();
    await expect(page.getByText('Basic cleanup · No AI tone', { exact: true })).toBeVisible();
    await expect(page.getByText('Cleanup off', { exact: true })).toHaveCount(0);
    const originsBeforeFinish = (await appleAvailabilityOrigins(page)).length;
    await captureSetupState(page, `basic-apple-done-before-${priorAppleChoice}-${testInfo.project.name}.png`);
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
    if (priorAppleChoice) {
      const finishOrigins = (await appleAvailabilityOrigins(page)).slice(originsBeforeFinish);
      expect(finishOrigins.filter(origin => origin === 'setup')).toEqual([]);
      expect(finishOrigins.filter(origin => origin === 'unknown')).toEqual([]);
      expect(await session.invoke('get_setting', { key: 'cleanup_default_model' })).not.toBe('apple-intelligence/system');
    }
    await captureSetupState(page, `basic-apple-done-after-${priorAppleChoice}-${testInfo.project.name}.png`);
  } finally {
    await restoreSetup(session, previous);
    await page.reload();
  }
});
}

test('Setup Finish re-checks Apple availability for AI cleanup (probe positive control)', async ({ page, session }) => {
  const previous = await snapshotSetup(session);
  try {
    await installAppleAvailabilityProbe(page);
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
    await page.evaluate(async () => {
      const { appleIntelligence, refreshAppleIntelligence } = await import('/src/lib/appleIntelligence.svelte.ts');
      await refreshAppleIntelligence();
      appleIntelligence.status = { state: 'available', available: true, message: 'Synthetic setup fixture' };
    });
    await page.getByRole('switch', { name: 'Clean up with Apple Intelligence', exact: true }).click();
    await page.getByRole('button', { name: 'Use local Transcription only' }).click();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    for (let i = 0; i < 6 && !(await page.getByText('Finish setup', { exact: true }).isVisible()); i++) {
      await page.getByRole('button', { name: 'Next', exact: true }).click();
    }
    await expect(page.getByText('Finish setup', { exact: true })).toBeVisible();
    const before = (await appleAvailabilityOrigins(page)).length;
    await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
    await expect.poll(async () => (await appleAvailabilityOrigins(page)).slice(before)).toContain('setup');
    expect((await appleAvailabilityOrigins(page)).slice(before)).not.toContain('unknown');
  } finally {
    await restoreSetup(session, previous);
    await page.reload();
  }
});
