import { test, expect } from './fixtures.mjs';

test('an older Models snapshot cannot restore a renamed provider', async ({ page, session }) => {
  const provider = {
    id: 'custom:87654321-1234-4234-8234-123456789123', name: 'Synthetic old',
    protocol: 'openai', base_url: 'http://127.0.0.1:1/v1', requires_key: false,
    supports_transcription: true, supports_cleanup: false, auth_header: null,
    extra_headers: {}, body_overrides: null, transcription_models: ['synthetic'], cleanup_models: [],
  };
  const original = (await session.invoke('get_all_settings')).custom_providers ?? [];
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Network.enable');
  let armed = false;
  const initiators = [];
  cdp.on('Network.requestWillBeSent', event => {
    if (!armed || !event.request.postData) return;
    try {
      if (JSON.parse(event.request.postData).command !== 'get_all_settings') return;
      const frames = [];
      for (let stack = event.initiator.stack; stack; stack = stack.parent) {
        for (const frame of stack.callFrames) frames.push(new URL(frame.url || 'http://localhost/').pathname);
      }
      initiators.push(frames);
    } catch {}
  });
  let release;
  const held = new Promise(resolve => { release = resolve; });
  let signal;
  const started = new Promise(resolve => { signal = resolve; });
  let finish;
  const finished = new Promise(resolve => { finish = resolve; });
  try {
    await session.invoke('save_setting', { key: 'custom_providers', value: [provider] });
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'General', exact: true })).toBeVisible();
    armed = true;
    let first = true;
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'get_all_settings' || !first) return route.continue();
      first = false;
      const response = await route.fetch();
      signal();
      await held;
      await route.fulfill({ response });
      finish();
    });
    await page.getByRole('button', { name: 'Models', exact: true }).click();
    await started;
    expect(initiators[0]?.some(path => path.endsWith('/ModelsSection.svelte'))).toBe(true);
    await page.getByRole('button', { name: 'Providers', exact: true }).click();
    await page.getByRole('button', { name: 'Edit Synthetic old', exact: true }).click();
    await page.getByLabel('Name', { exact: true }).fill('Synthetic new');
    await page.getByRole('button', { name: 'Save provider', exact: true }).click();
    await expect(page.getByRole('status').filter({ hasText: 'Provider saved' })).toBeVisible();
    await expect(page.getByRole('form', { name: 'Custom provider editor' })).toBeHidden();
    await expect(page.getByRole('button', { name: 'Remove Synthetic new', exact: true })).toBeVisible();
    release();
    await finished;
    await page.waitForFunction(async () => {
      const { frontendIpcActivity } = await import('/src/lib/diagnostics.ts');
      return frontendIpcActivity.snapshot().find(metric => metric.command === 'get_all_settings')?.currently_running === 0;
    });
    await expect(page.getByRole('button', { name: 'Remove Synthetic new', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Remove Synthetic old', exact: true })).toHaveCount(0);
  } finally {
    release();
    try {
      await page.unroute('**/__verenu_dev/invoke');
    } finally {
      try {
        await session.invoke('save_setting', { key: 'custom_providers', value: original });
      } finally {
        await cdp.detach();
      }
    }
  }
});
