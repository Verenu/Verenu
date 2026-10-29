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
      } catch {}
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

test('A transcription-only preset explains why an AI cleanup level is off and links back to models', async ({ page, session }) => {
  const previous = await snapshotSetup(session);
  try {
    await page.goto('about:blank');
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: 'medium' });
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: true });
    await session.invoke('save_setting', { key: 'transcription_provider', value: 'local' });
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'local/whisper-small' });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 4, provider: 'local' } });
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    const progressLoaded = page.waitForResponse(response => response.url().endsWith('/__verenu_dev/invoke')
      && response.request().postDataJSON()?.command === 'get_setting'
      && response.request().postDataJSON()?.args?.key === 'setup_progress');
    await page.goto(session.access.localAccessUrl);
    await progressLoaded;
    await expect(page.getByRole('button', { name: 'Use local Transcription only' })).toBeAttached();
    await page.evaluate(async () => {
      const { localSttStore } = await import('/src/lib/localSttStore.svelte.ts');
      const { invoke } = await import('/src/lib/tauri.ts');
      localSttStore.models = (await invoke('list_local_stt_models')).map(model => ({ ...model, is_downloaded: true, is_downloading: false }));
    });
    await page.getByRole('button', { name: 'Use local Transcription only' }).click();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await page.getByRole('button', { name: /^Light / }).click();
    await expect(page.getByRole('status').filter({ hasText: 'Cleanup is off with your transcription-only model choice.' })).toBeVisible();
    await page.getByRole('button', { name: 'Choose a preset with a cleanup model' }).click();
    await expect(page.getByRole('button', { name: 'Use local Transcription only' })).toBeVisible();
  } finally {
    await restoreSetup(session, previous);
    await page.reload();
  }
});

test('Setup preserves an agent change and offers to reload on a save conflict', async ({ page, session }) => {
  const previous = await snapshotSetup(session);
  let release;
  const held = new Promise(resolve => { release = resolve; });
  let started;
  const pending = new Promise(resolve => { started = resolve; });
  let reloaded = false;
  let captureReloaded;
  const reloadedRequest = new Promise(resolve => { captureReloaded = resolve; });
  try {
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON().command !== 'get_api_key_status') return route.fallback();
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ openai: true, local: true }) });
    });
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 4, provider: 'local' } });
    await session.invoke('save_setting', { key: 'transcription_provider', value: 'local' });
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'local/whisper-small' });
    await page.reload();
    await expect(page.getByRole('button', { name: 'Use local Transcription only' })).toBeAttached();
    await page.evaluate(async () => {
      const { localSttStore } = await import('/src/lib/localSttStore.svelte.ts');
      const { invoke } = await import('/src/lib/tauri.ts');
      localSttStore.models = (await invoke('list_local_stt_models')).map(model => ({ ...model, is_downloaded: true, is_downloading: false }));
    });
    await page.getByRole('button', { name: 'Use local Transcription only' }).click();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await page.getByRole('button', { name: /^Light / }).click();
    for (let i = 0; i < 6 && !(await page.getByText('Finish setup', { exact: true }).isVisible()); i++) {
      await page.getByRole('button', { name: 'Next', exact: true }).click();
    }
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command !== 'save_model_settings_if_unchanged') return route.fallback();
      if (reloaded) captureReloaded(request);
      started();
      await held;
      // The browser bridge blocks protected Setup writes; simulate the native conflict response.
      await route.fulfill({ status: 409, contentType: 'application/json', body: JSON.stringify({ error: 'Settings changed while you were editing. Reload them before saving.' }) });
    });
    await captureSetupState(page, `setup-conflict-before-${test.info().project.name}.png`);
    await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
    await pending;
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: false });
    await session.invoke('save_setting', { key: 'noise_reduction', value: false });
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'openai/whisper-1' });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['cleanup_enabled']));
    await expect.poll(() => page.evaluate(async () => (await import('/src/lib/stores.svelte.ts')).appStore.cleanupEnabled)).toBe(false);
    release();
    await expect(page.getByRole('alert')).toContainText('Settings changed outside setup');
    await expect(page.getByRole('button', { name: 'Reload setup' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Finish setup' })).toHaveCount(0);
    await captureSetupState(page, `setup-conflict-after-${test.info().project.name}.png`);
    const persisted = await session.invoke('get_setting', { key: 'cleanup_enabled' });
    expect(persisted).toBe(false);
    await expect.poll(() => page.evaluate(async () => (await import('/src/lib/stores.svelte.ts')).appStore.cleanupEnabled)).toBe(persisted);
    await page.getByRole('button', { name: 'Reload setup' }).click();
    await expect(page.getByText('Cleanup off', { exact: true })).toBeVisible();
    await page.reload();
    await expect(page.getByText('Cleanup off', { exact: true })).toBeVisible();
    reloaded = true;
    await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
    const retry = await reloadedRequest;
    expect(retry.args.values.noise_reduction).toBe(false);
    expect(retry.args.values.transcription_default_model).toBe('openai/whisper-1');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await restoreSetup(session, previous);
    await page.reload();
  }
});

test('resuming setup preserves saved model settings after an interrupted session', async ({ page, session }) => {
  const previous = await snapshotSetup(session);
  let capture;
  const requestCaptured = new Promise(resolve => { capture = resolve; });
  try {
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 9, provider: 'local' } });
    await session.invoke('save_setting', { key: 'transcription_provider', value: 'local' });
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'openai/whisper-1' });
    await session.invoke('save_setting', { key: 'noise_reduction', value: false });
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON().command !== 'get_api_key_status') return route.fallback();
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ openai: true, local: true }) });
    });
    await page.reload();
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command !== 'save_model_settings_if_unchanged') return route.fallback();
      capture(request);
      await route.fulfill({ status: 409, contentType: 'application/json', body: JSON.stringify({ error: 'Settings changed while you were editing. Reload them before saving.' }) });
    });
    await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
    const request = await requestCaptured;
    expect(request.args.values.transcription_default_model).toBe('openai/whisper-1');
    expect(request.args.values.noise_reduction).toBe(false);
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    await restoreSetup(session, previous);
    await page.reload();
  }
});

test('Setup applies the selected speaker audio behavior', async ({ page, session }) => {
  const previous = await snapshotSetup(session);
  let capture;
  const requestCaptured = new Promise(resolve => { capture = resolve; });
  try {
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 9, provider: 'local' } });
    await session.invoke('save_setting', { key: 'transcription_provider', value: 'local' });
    await session.invoke('save_setting', { key: 'mute_audio', value: true });
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON().command !== 'get_all_settings') return route.fallback();
      const response = await route.fetch();
      await route.fulfill({ response, json: { ...await response.json(), pause_media_during_dictation: false } });
    });
    await page.reload();
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command !== 'save_model_settings_if_unchanged') return route.fallback();
      capture(request);
      await route.fulfill({ status: 409, contentType: 'application/json', body: JSON.stringify({ error: 'Settings changed while you were editing.' }) });
    });
    await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
    const request = await requestCaptured;
    expect(request.args.expected.pause_media_during_dictation).toBe(false);
    expect(request.args.values.pause_media_during_dictation).toBe(true);
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    await restoreSetup(session, previous);
    await page.reload();
  }
});

for (const progress of [null, { step: 4, provider: 'local' }]) {
  test(`Setup retains an existing model from ${progress ? 'the models step' : 'the first launch'}`, async ({ page, session }) => {
    const previous = await snapshotSetup(session);
    let capture;
    const requestCaptured = new Promise(resolve => { capture = resolve; });
    try {
      await session.invoke('save_setting', { key: 'setup_complete', value: false });
      await session.invoke('save_setting', { key: 'setup_progress', value: progress });
      await session.invoke('save_setting', { key: 'transcription_provider', value: 'local' });
      await session.invoke('save_setting', { key: 'transcription_default_model', value: 'local/whisper-small' });
      await session.invoke('save_setting', { key: 'cleanup_enabled', value: false });
      await page.reload();
      if (!progress) {
        await page.getByRole('button', { name: 'Get Started' }).click();
        await page.getByRole('button', { name: 'Next', exact: true }).click();
        await page.getByRole('button', { name: 'Next', exact: true }).click();
        await page.getByRole('button', { name: 'Continue', exact: true }).click();
      }
      for (let i = 0; i < 6 && !(await page.getByRole('button', { name: 'Finish setup', exact: true }).isVisible()); i++) {
        await page.getByRole('button', { name: 'Next', exact: true }).click();
      }
      await expect(page.getByText("Your selected on-device speech model isn't installed yet.")).toBeVisible();
      await page.evaluate(async () => {
        const { localSttStore } = await import('/src/lib/localSttStore.svelte.ts');
        localSttStore.models = localSttStore.models.map(model => ({ ...model, is_downloaded: model.id === 'whisper-small' }));
      });
      await expect(page.getByText("Your selected on-device speech model isn't installed yet.")).toHaveCount(0);
      await page.route('**/__verenu_dev/invoke', async route => {
        const request = route.request().postDataJSON();
        if (request.command !== 'save_model_settings_if_unchanged') return route.fallback();
        capture(request);
        await route.fulfill({ status: 409, contentType: 'application/json', body: JSON.stringify({ error: 'Settings changed while you were editing.' }) });
      });
      await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
      expect((await requestCaptured).args.values.transcription_default_model).toBe('local/whisper-small');
    } finally {
      await page.unrouteAll({ behavior: 'wait' });
      await restoreSetup(session, previous);
      await page.reload();
    }
  });
}

test('Setup uses the saved model provider for the key screen and key save', async ({ page, session }) => {
  const previous = await snapshotSetup(session);
  let capture;
  const keyRequest = new Promise(resolve => { capture = resolve; });
  try {
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 3, provider: 'local' } });
    await session.invoke('save_setting', { key: 'transcription_provider', value: 'local' });
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'openai/whisper-1' });
    await page.reload();
    await expect(page.getByText('Do you already have a OpenAI API key?')).toBeVisible();
    await page.getByRole('button', { name: /Yes, I have/ }).click();
    await page.getByRole('textbox', { name: 'API key' }).fill('synthetic-test-key');
    let rejectSave = true;
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command !== 'save_api_key') return route.fallback();
      if (rejectSave) {
        rejectSave = false;
        return route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Credential store unavailable' }) });
      }
      capture(request);
      await route.fulfill({ status: 200, contentType: 'application/json', body: 'null' });
    });
    await page.getByRole('button', { name: 'Save Key' }).click();
    await expect(page.getByRole('alert')).toContainText('Could not save your API key');
    await page.getByRole('button', { name: 'Save Key' }).click();
    expect((await keyRequest).args.provider).toBe('openai');
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    await restoreSetup(session, previous);
    await page.reload();
  }
});

test('returning to the key screen uses the automatically selected cross-provider model', async ({ page, session }) => {
  const previous = await snapshotSetup(session);
  try {
    const snapshot = { ...previous };
    for (const key of ['transcription_model', 'transcription_default_model', 'transcription_models_by_provider',
      'transcription_fallback_models', 'dual_transcription_enabled', 'cleanup_provider', 'cleanup_model',
      'cleanup_default_model', 'cleanup_models_by_provider', 'cleanup_fallback_models']) {
      snapshot[key] = null;
    }
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 4, provider: 'groq' } });
    await session.invoke('save_setting', { key: 'transcription_provider', value: 'groq' });
    snapshot.transcription_provider = 'groq';
    let groqKey = true;
    await page.route('**/__verenu_dev/invoke', async route => {
      const command = route.request().postDataJSON().command;
      if (command === 'get_all_settings') return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(snapshot) });
      if (command === 'get_api_key_status') return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ groq: groqKey, openai: true, local: true }) });
      return route.fallback();
    });
    await page.reload();
    await expect(page.locator('[aria-pressed="true"]').first()).toBeVisible();
    groqKey = false;
    const refreshed = page.waitForResponse(response => response.url().endsWith('/__verenu_dev/invoke') && response.request().postDataJSON().command === 'get_api_key_status');
    await page.evaluate(() => window.dispatchEvent(new Event('verenu:api-key-deleted')));
    await refreshed;
    await expect(page.locator('[aria-pressed="true"]').first()).toBeVisible();
    await page.getByRole('button', { name: 'Back', exact: true }).click();
    await expect(page.getByText('Do you already have a OpenAI API key?')).toBeVisible();
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    await restoreSetup(session, previous);
    await page.reload();
  }
});

test('changing provider clears the previous model preset before the key screen', async ({ page, session }) => {
  const previous = await snapshotSetup(session);
  try {
    const snapshot = { ...previous, transcription_provider: 'groq' };
    for (const key of ['transcription_model', 'transcription_default_model', 'transcription_models_by_provider',
      'transcription_fallback_models', 'dual_transcription_enabled', 'cleanup_provider', 'cleanup_model',
      'cleanup_default_model', 'cleanup_models_by_provider', 'cleanup_fallback_models']) snapshot[key] = null;
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 4, provider: 'groq' } });
    await page.route('**/__verenu_dev/invoke', async route => {
      const command = route.request().postDataJSON().command;
      if (command === 'get_all_settings') return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(snapshot) });
      if (command === 'get_api_key_status') return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ groq: true, openai: true, local: true }) });
      return route.fallback();
    });
    await page.reload();
    await expect(page.locator('[aria-pressed="true"]').first()).toBeVisible();
    await page.getByRole('button', { name: 'Back', exact: true }).click();
    await page.getByRole('button', { name: 'Back', exact: true }).click();
    await page.getByRole('button', { name: /OpenAI/ }).click();
    await page.getByRole('button', { name: 'Next', exact: true }).click();
    await expect(page.getByText('Do you already have a OpenAI API key?')).toBeVisible();
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    await restoreSetup(session, previous);
    await page.reload();
  }
});

for (const provider of ['groq', 'openrouter']) {
  test(`Setup preserves disabled cleanup and an existing ${provider} provider without model fields`, async ({ page, session }) => {
    const previous = await snapshotSetup(session);
    let capture;
    const requestCaptured = new Promise(resolve => { capture = resolve; });
    try {
      const snapshot = { ...previous, transcription_provider: provider, cleanup_enabled: false };
      for (const key of ['transcription_model', 'transcription_default_model', 'transcription_models_by_provider',
        'transcription_fallback_models', 'dual_transcription_enabled', 'cleanup_provider', 'cleanup_model',
        'cleanup_default_model', 'cleanup_models_by_provider', 'cleanup_fallback_models']) {
        snapshot[key] = null;
      }
      await session.invoke('save_setting', { key: 'setup_complete', value: false });
      await session.invoke('save_setting', { key: 'setup_progress', value: { step: 9, provider: 'groq' } });
      await session.invoke('save_setting', { key: 'transcription_provider', value: provider });
      await session.invoke('save_setting', { key: 'cleanup_enabled', value: false });
      await page.route('**/__verenu_dev/invoke', async route => {
        const request = route.request().postDataJSON();
        if (request.command === 'get_all_settings') return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(snapshot) });
        if (request.command === 'get_api_key_status') return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ groq: true, local: true }) });
        if (request.command !== 'save_model_settings_if_unchanged') return route.fallback();
        capture(request);
        await route.fulfill({ status: 409, contentType: 'application/json', body: JSON.stringify({ error: 'Settings changed while you were editing.' }) });
      });
      await page.reload();
      await expect(page.getByText('Cleanup off', { exact: true })).toBeVisible();
      await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
      const values = (await requestCaptured).args.values;
      expect(values.cleanup_enabled).toBe(false);
      expect(values.transcription_provider).toBe(provider);
      if (provider === 'openrouter') expect(values.transcription_default_model).toBeNull();
    } finally {
      await page.unrouteAll({ behavior: 'wait' });
      await restoreSetup(session, previous);
      await page.reload();
    }
  });
}

test('finishing setup refreshes the shared transcription model', async ({ page, session }) => {
  const previous = await snapshotSetup(session);
  try {
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 9, provider: 'local' } });
    await session.invoke('save_setting', { key: 'transcription_provider', value: 'local' });
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'groq/whisper-large-v3' });
    await page.reload();
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command === 'initialize_autostart' || (request.command === 'save_setting' && request.args.key === 'setup_complete')) {
        return route.fulfill({ status: 200, contentType: 'application/json', body: 'null' });
      }
      if (request.command !== 'save_model_settings_if_unchanged') return route.fallback();
      await session.invoke('save_setting', { key: 'transcription_provider', value: 'local' });
      await session.invoke('save_setting', { key: 'transcription_default_model', value: 'local/parakeet-v3' });
      await route.fulfill({ status: 200, contentType: 'application/json', body: 'null' });
    });
    await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
    await expect.poll(() => page.evaluate(async () => (await import('/src/lib/transcriptionModelStore.svelte.ts')).transcriptionModelStore.defaultModel)).toBe('local/parakeet-v3');
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    await restoreSetup(session, previous);
    await page.reload();
  }
});

test('finishing setup does not restore an older appearance after an agent change', async ({ page, session }) => {
  const previous = await snapshotSetup(session);
  let release;
  let captured;
  const held = new Promise(resolve => { release = resolve; });
  const pending = new Promise(resolve => { captured = resolve; });
  try {
    await session.invoke('save_setting', { key: 'appearance_mode', value: 'light' });
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 9, provider: 'local' } });
    await page.reload();
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request.command === 'save_model_settings_if_unchanged' || (request.command === 'save_setting' && request.args.key === 'setup_complete')) {
        return route.fulfill({ status: 200, contentType: 'application/json', body: 'null' });
      }
      if (request.command !== 'initialize_autostart') return route.fallback();
      captured();
      await held;
      await route.fulfill({ status: 200, contentType: 'application/json', body: 'null' });
    });
    await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
    await pending;
    await session.invoke('save_setting', { key: 'appearance_mode', value: 'dark' });
    await page.evaluate(async () => (await import('/src/lib/tauri.ts')).emit('verenu:agent-settings-changed', ['appearance_mode']));
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    release();
    await expect(page.locator('[data-debug-id="nav.settings"]')).toBeVisible();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await restoreSetup(session, previous);
    await page.reload();
  }
});

for (const priorAppleChoice of [false, true]) {
test(`Setup keeps Basic cleanup on after a transcription-only preset${priorAppleChoice ? ' and an unavailable Apple choice' : ''}`, async ({ page, session }, testInfo) => {
  const previous = await snapshotSetup(session);
  try {
    await installAppleAvailabilityProbe(page);
    await page.goto('about:blank');
    await session.invoke('save_setting', { key: 'cleanup_intensity', value: 'rules' });
    await session.invoke('save_setting', { key: 'cleanup_enabled', value: true });
    await session.invoke('save_setting', { key: 'transcription_provider', value: 'local' });
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'local/whisper-small' });
    await session.invoke('save_setting', { key: 'setup_progress', value: { step: 4, provider: 'local' } });
    await session.invoke('save_setting', { key: 'setup_complete', value: false });
    const progressLoaded = page.waitForResponse(response => response.url().endsWith('/__verenu_dev/invoke')
      && response.request().postDataJSON()?.command === 'get_setting'
      && response.request().postDataJSON()?.args?.key === 'setup_progress');
    await page.goto(session.access.localAccessUrl);
    await progressLoaded;

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

    await expect(page.getByRole('button', { name: /^Basic / })).toHaveCount(0);
    for (const level of ['Off', 'Light', 'Medium', 'Strong']) {
      await expect(page.getByRole('button', { name: new RegExp(`^${level} `) })).toBeVisible();
    }
    await expect(page.getByText('Your previous Basic setting is still active', { exact: false })).toBeVisible();
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
      return payload?.command === 'save_model_settings_if_unchanged';
    });
    await page.getByRole('button', { name: 'Finish setup', exact: true }).click();
    const saveResponse = await protectedSetting;
    expect(saveResponse.status()).toBe(403);
    const saveRequest = saveResponse.request().postDataJSON().args;
    expect(saveRequest.expected.cleanup_intensity).toBe('rules');
    expect(saveRequest.expected.cleanup_enabled).toBe(true);
    expect(saveRequest.values.cleanup_intensity).toBe('rules');
    expect(saveRequest.values.cleanup_enabled).toBe(true);
    expect(saveRequest.values.cleanup_default_model).not.toBe('apple-intelligence/system');
    await expect(page.getByRole('alert').filter({ hasText: 'Could not save your setup choices.' })).toContainText('This setting is protected in dev sessions');
    expect(await session.invoke('get_setting', { key: 'cleanup_enabled' })).toBe(true);
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
    await session.invoke('save_setting', { key: 'transcription_provider', value: 'local' });
    await session.invoke('save_setting', { key: 'transcription_default_model', value: 'local/whisper-small' });
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
