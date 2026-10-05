import { test, expect } from './fixtures.mjs';

// Dev sessions prohibit sync networking. These checks reveal the Sync page and
// feed it synthetic status snapshots, so they cover presentation and local
// interaction only. Native device tests separately exercise real pairing.
const ago = (minutes) => new Date(Date.now() - minutes * 60_000).toISOString();
const snapshot = (overrides = {}) => ({
  this_device: { uuid: 'synthetic-this', name: 'Studio PC', port: 48211, tailscale_ips: ['100.101.102.103'] },
  listener_active: true,
  pairing: null,
  discovered: [{ uuid: 'synthetic-near', name: 'Kitchen laptop', addresses: [], port: 1, paired: false, last_seen_ms: 0 }],
  peers: [
    { uuid: 'synthetic-p1', name: 'Pixel Fold', added_at: ago(900), last_sync_at: ago(3), state: 'synced', error: null, online: true, connection_address: '100.64.0.7:48211' },
    { uuid: 'synthetic-p2', name: 'Synthetic laptop with an extremely long device name that must wrap cleanly', added_at: ago(900), last_sync_at: ago(240), state: 'error', error: 'Could not reach this device.', online: false, connection_address: null },
  ],
  last_error_hint: null,
  ...overrides,
});

async function openSync(page, status = snapshot()) {
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.evaluate(async (next) => {
    const { appStore } = await import('/src/lib/stores.ts');
    const { syncStore } = await import('/src/lib/syncStore.svelte.ts');
    appStore.syncEnabled = true;
    appStore.settingsSection = 'sync';
    syncStore.status = next;
    syncStore.loaded = true;
  }, status);
  await expect(page.getByRole('heading', { name: 'Sync', level: 2 })).toBeVisible();
}

const setStatus = (page, status) => page.evaluate(async (next) => {
  const { syncStore } = await import('/src/lib/syncStore.svelte.ts');
  syncStore.status = next;
}, status);

const fitsWidth = (locator) => locator.evaluate((element) => {
  const box = element.getBoundingClientRect();
  const overflowing = [...element.querySelectorAll('*')].filter((child) => {
    const rect = child.getBoundingClientRect();
    return rect.width > 0 && (rect.right > box.right + 1 || rect.left < box.left - 1);
  });
  return { scrolls: element.scrollWidth > element.clientWidth + 1, overflowing: overflowing.map((n) => n.className || n.tagName) };
});

test('Tailscale setup is a stepwise disclosure that copies, validates, and fits', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await openSync(page);
  const toggle = page.getByRole('button', { name: /Connect through Tailscale/ });
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(page.locator('details')).toHaveCount(0);
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  const panel = page.locator('#sync-setup-panel');
  await expect(panel.locator('ol.steps > li')).toHaveCount(3);
  // The raw connection string stays out of the way unless copying fails.
  await expect(panel.getByText(/verenu-sync:\/\//)).toHaveCount(0);
  await expect(panel.getByText('100.101.102.103')).toBeVisible();

  const pair = panel.getByRole('button', { name: 'Pair', exact: true });
  await expect(pair).toBeDisabled();
  await panel.getByLabel("Paste the other device's details").fill('verenu-sync://synthetic');
  await expect(pair).toBeEnabled();

  await panel.getByRole('button', { name: 'Copy details' }).click();
  await expect(panel.getByRole('button', { name: 'Copied' })).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('verenu-sync://synthetic-this@100.101.102.103:48211');
  await expect(panel.getByRole('button', { name: 'Copy details' })).toBeVisible({ timeout: 4000 });

  await panel.getByRole('button', { name: /Tips and troubleshooting/ }).click();
  await expect(panel.getByText(/three or more devices/)).toBeVisible();
  await expect(panel.getByText(/Android can pause Verenu/)).toBeVisible();
  const fit = await fitsWidth(page.locator('[data-setting-target="sync-tailscale"]'));
  expect(fit, 'Setup fits its container').toEqual({ scrolls: false, overflowing: [] });

  await toggle.click();
  await expect(panel).toHaveCount(0);
});

test('Setup asks for the address when none is detected', async ({ page }) => {
  await openSync(page, snapshot({ this_device: { uuid: 'synthetic-this', name: 'Studio PC', port: 48211 } }));
  await page.getByRole('button', { name: /Connect through Tailscale/ }).click();
  const panel = page.locator('#sync-setup-panel');
  const copy = panel.getByRole('button', { name: 'Copy details' });
  await expect(copy).toBeDisabled();
  await expect(panel.getByText(/could not detect the address/)).toBeVisible();
  await panel.getByLabel("This device's Tailscale IPv4 address").fill('100.90.80.70');
  await expect(copy).toBeEnabled();
});

test('Connection editor opens inside its device, closes with Escape, and returns focus', async ({ page }) => {
  await openSync(page);
  const device = page.locator('.device', { hasText: 'Pixel Fold' });
  const button = device.getByRole('button', { name: 'Connection' });
  await expect(button).toHaveAttribute('aria-expanded', 'false');
  await button.click();
  await expect(button).toHaveAttribute('aria-expanded', 'true');
  const input = device.getByLabel('How to reach Pixel Fold');
  await expect(input).toBeFocused();
  await expect(input).toHaveValue('100.64.0.7:48211');
  expect(await fitsWidth(device)).toEqual({ scrolls: false, overflowing: [] });
  await page.keyboard.press('Escape');
  await expect(input).toHaveCount(0);
  await expect(button).toBeFocused();
  await expect(button).toHaveAttribute('aria-expanded', 'false');
});

test('Device rows, long names, errors and status fit at this width', async ({ page }) => {
  await openSync(page);
  // Two paired devices plus one nearby device.
  await expect(page.locator('.device')).toHaveCount(3);
  for (const device of await page.locator('.device').all()) {
    expect(await fitsWidth(device)).toEqual({ scrolls: false, overflowing: [] });
  }
  await expect(page.getByText('Up to date')).toBeVisible();
  await expect(page.getByText('Sync failed')).toBeVisible();
  await expect(page.getByText(/Last synced 3 min ago/)).toBeVisible();
});

test('Pairing dialog walks through connecting, code, and verifying', async ({ page }) => {
  await openSync(page);
  const pairing = (phase, code) => snapshot({ pairing: { kind: 'outgoing', phase, peer_uuid: 'synthetic-near', peer_name: 'Kitchen laptop', code, error: null } });
  await setStatus(page, pairing('connecting', null));
  const dialog = page.getByRole('dialog', { name: 'Pairing with Kitchen laptop' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText('Contacting Kitchen laptop…')).toBeVisible();
  await expect(dialog.locator('.digit.placeholder')).toHaveCount(6);
  await setStatus(page, pairing('waiting_for_code', '123456'));
  await expect(dialog.getByRole('img', { name: 'Pairing code 1 2 3 4 5 6' })).toBeVisible();
  await expect(dialog.locator('.digit:not(.placeholder)')).toHaveText(['1', '2', '3', '4', '5', '6']);
  await expect(dialog.getByText(/Waiting for Kitchen laptop/)).toBeVisible();
  expect(await fitsWidth(dialog)).toEqual({ scrolls: false, overflowing: [] });
  // The dialog must sit above the sidebar rail and inside the window.
  await page.waitForTimeout(400);
  const placement = await dialog.evaluate((element) => {
    const box = element.getBoundingClientRect();
    const hit = (x) => element.contains(document.elementFromPoint(x, box.top + box.height / 2));
    return { inside: box.left >= 0 && box.right <= innerWidth, leftEdgeVisible: hit(box.left + 4), rightEdgeVisible: hit(box.right - 4) };
  });
  expect(placement).toEqual({ inside: true, leftEdgeVisible: true, rightEdgeVisible: true });
  await setStatus(page, pairing('verifying', '123456'));
  await expect(dialog.getByText('Checking the code…')).toBeVisible();
  await expect(dialog.locator('.phase-seg.on')).toHaveCount(3);
  await setStatus(page, snapshot());
  await expect(dialog).toHaveCount(0);
});

test('A newly paired device announces itself and offers the return route', async ({ page }) => {
  await openSync(page, snapshot({ peers: [] }));
  await expect(page.getByText('Nothing paired yet')).toBeVisible();
  await setStatus(page, snapshot({ peers: [{ uuid: 'synthetic-new', name: 'Pixel Fold', added_at: ago(0), last_sync_at: null, state: 'connecting', error: null, online: false, connection_address: '100.64.0.7:48211' }] }));
  await expect(page.getByRole('status').filter({ hasText: 'Paired with Pixel Fold.' })).toBeVisible();
  const device = page.locator('.device', { hasText: 'Pixel Fold' });
  await expect(device.getByText(/One more step so Pixel Fold can reach you/)).toBeVisible();
  await device.getByRole('button', { name: 'Done' }).click();
  await expect(device.getByText(/One more step/)).toHaveCount(0);
});

test('Reduced motion stops looping indicators and keeps status readable', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await openSync(page, snapshot({ peers: [{ uuid: 'synthetic-p1', name: 'Pixel Fold', added_at: ago(9), last_sync_at: ago(1), state: 'syncing', error: null, online: true, connection_address: null }] }));
  const spinner = page.locator('.sync-glyph.spin');
  await expect(spinner).toHaveCount(1);
  expect(await spinner.evaluate((node) => getComputedStyle(node).animationName)).toBe('none');
  expect(await page.locator('.pill.syncing .pill-dot').evaluate((node) => getComputedStyle(node).animationName)).toBe('none');
  await expect(page.getByText('Syncing', { exact: true }).first()).toBeVisible();
});

test('Motion plays when motion is allowed', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await openSync(page, snapshot({ peers: [{ uuid: 'synthetic-p1', name: 'Pixel Fold', added_at: ago(9), last_sync_at: ago(1), state: 'syncing', error: null, online: true, connection_address: null }] }));
  expect(await page.locator('.sync-glyph.spin').evaluate((node) => getComputedStyle(node).animationName)).toMatch(/sync-spin$/);
  const toggle = page.getByRole('button', { name: /Connect through Tailscale/ });
  await toggle.click();
  // The panel is mid-slide shortly after opening, then settles at full height.
  const heights = await page.evaluate(async () => {
    const panel = document.querySelector('#sync-setup-panel');
    const samples = [];
    for (let i = 0; i < 12; i += 1) {
      samples.push(Math.round(panel.getBoundingClientRect().height));
      await new Promise((resolve) => setTimeout(resolve, 30));
    }
    return samples;
  });
  expect(heights[0], 'starts collapsed').toBeLessThan(heights.at(-1));
  expect(new Set(heights).size, 'height changes over several frames').toBeGreaterThan(2);
});
