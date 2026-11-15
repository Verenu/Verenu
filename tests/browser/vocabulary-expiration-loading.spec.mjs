import { test, expect } from './fixtures.mjs';
import fs from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';

test('vocabulary expiration waits for its saved policy', async ({ page, session }, testInfo) => {
  let release;
  const blocked = new Promise(resolve => { release = resolve; });
  let statusRequested = false;
  await session.invoke('set_vocabulary_expiration', { days: 60 });
  try {
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request?.command === 'get_vocabulary_expiration') {
        statusRequested = true;
        await blocked;
      }
      await route.continue();
    });
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.text"]').click();
    await expect.poll(() => statusRequested).toBe(true);
    const row = page.locator('[data-setting-target="text-vocabulary-expiration"]');
    await expect(row.getByRole('status')).toHaveText('Loading…');
    await expect(row.getByRole('button', { name: 'Delete unused vocabulary after' })).toHaveCount(0);
    await row.scrollIntoViewIfNeeded();
    await expect.poll(() => row.evaluate(element => {
      let opacity = 1;
      for (let node = element; node; node = node.parentElement) opacity *= Number(getComputedStyle(node).opacity);
      return opacity;
    })).toBeGreaterThan(0.99);
    await page.screenshot({ path: testInfo.outputPath('vocabulary-policy-loading.png') });
    release();
    await expect(row.getByRole('button', { name: 'Delete unused vocabulary after' })).toHaveText('60 days');
    await page.screenshot({ path: testInfo.outputPath('vocabulary-policy-loaded.png') });
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('set_vocabulary_expiration', { days: null });
  }
});

test('vocabulary expiration reports an unavailable policy instead of Off', async ({ page, session }) => {
  await session.invoke('set_vocabulary_expiration', { days: 60 });
  try {
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request?.command === 'get_vocabulary_expiration') {
        await route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Synthetic status failure' }) });
      } else {
        await route.continue();
      }
    });
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.text"]').click();
    const row = page.locator('[data-setting-target="text-vocabulary-expiration"]');
    await expect(row.getByRole('status')).toHaveText('Unavailable');
    await expect(row.getByRole('button', { name: 'Delete unused vocabulary after' })).toHaveCount(0);
    await expect(page.getByRole('alert')).toBeVisible();
    await page.unrouteAll({ behavior: 'wait' });
    await page.evaluate(async () => {
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('verenu:sync-status', { state: 'synced' });
    });
    await expect(row.getByRole('button', { name: 'Delete unused vocabulary after' })).toHaveText('60 days');
    await expect(page.getByRole('alert')).toHaveCount(0);
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('set_vocabulary_expiration', { days: null });
  }
});

test('an older sync response cannot overwrite a newer policy', async ({ page, session }) => {
  await session.invoke('set_vocabulary_expiration', { days: null });
  let release;
  const blocked = new Promise(resolve => { release = resolve; });
  let captured = false;
  let claimed = false;
  let delivered = false;
  try {
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.text"]').click();
    const select = page.getByRole('button', { name: 'Delete unused vocabulary after' });
    await expect(select).toHaveText('Off');
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request?.command === 'get_vocabulary_expiration' && !claimed) {
        claimed = true;
        const response = await route.fetch();
        captured = true;
        await blocked;
        await route.fulfill({ response });
        delivered = true;
      } else {
        await route.continue();
      }
    });
    await page.evaluate(async () => {
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('verenu:sync-status', { state: 'synced' });
    });
    await expect.poll(() => captured).toBe(true);
    await select.click();
    await page.getByRole('option', { name: '60 days', exact: true }).click();
    await expect(select).toHaveText('60 days');
    release();
    await expect.poll(() => delivered).toBe(true);
    await expect(select).toHaveText('60 days');
    expect((await session.invoke('get_vocabulary_expiration')).days).toBe(60);
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('set_vocabulary_expiration', { days: null });
  }
});

test('a late initial response cannot overwrite a newer sync refresh', async ({ page, session }) => {
  await session.invoke('set_vocabulary_expiration', { days: null });
  let release;
  const blocked = new Promise(resolve => { release = resolve; });
  let captured = false;
  let claimed = false;
  let delivered = false;
  try {
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request?.command === 'get_vocabulary_expiration' && !claimed) {
        claimed = true;
        const response = await route.fetch();
        captured = true;
        await blocked;
        await route.fulfill({ response });
        delivered = true;
      } else {
        await route.continue();
      }
    });
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.text"]').click();
    await expect.poll(() => captured).toBe(true);
    await session.invoke('set_vocabulary_expiration', { days: 60 });
    await page.evaluate(async () => {
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('verenu:sync-status', { state: 'synced' });
    });
    const select = page.getByRole('button', { name: 'Delete unused vocabulary after' });
    await expect(select).toHaveText('60 days');
    release();
    await expect.poll(() => delivered).toBe(true);
    await expect(select).toHaveText('60 days');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('set_vocabulary_expiration', { days: null });
  }
});

test('a failed edit does not leave a stale policy visible after sync', async ({ page, session }) => {
  await session.invoke('set_vocabulary_expiration', { days: 60 });
  let release;
  const blocked = new Promise(resolve => { release = resolve; });
  let captured = false;
  let claimed = false;
  let delivered = false;
  try {
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.text"]').click();
    const row = page.locator('[data-setting-target="text-vocabulary-expiration"]');
    const select = row.getByRole('button', { name: 'Delete unused vocabulary after' });
    await expect(select).toHaveText('60 days');
    await session.invoke('set_vocabulary_expiration', { days: 7 });
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request?.command === 'get_vocabulary_expiration' && !claimed) {
        claimed = true;
        const response = await route.fetch();
        captured = true;
        await blocked;
        await route.fulfill({ response });
        delivered = true;
      } else if (request?.command === 'set_vocabulary_expiration') {
        await route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Synthetic save failure' }) });
      } else {
        await route.continue();
      }
    });
    await page.evaluate(async () => {
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('verenu:sync-status', { state: 'synced' });
    });
    await expect.poll(() => captured).toBe(true);
    await select.click();
    await page.getByRole('option', { name: '8 days', exact: true }).click();
    await expect(row.getByRole('status')).toHaveText('Unavailable');
    release();
    await expect.poll(() => delivered).toBe(true);
    await expect(select).toHaveCount(0);
    expect((await session.invoke('get_vocabulary_expiration')).days).toBe(7);
    await expect(page.getByRole('alert')).toContainText('Could not save vocabulary expiration');
  } finally {
    release();
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('set_vocabulary_expiration', { days: null });
  }
});

test('a restored item is not offered again when its refresh fails', async ({ page, session }) => {
  const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
  const manifest = JSON.parse(await fs.readFile(path.join(directory, 'session.json'), 'utf8'));
  expect(manifest.syntheticSeed).toBe(true);
  const db = new DatabaseSync(path.join(directory, 'data', 'verenu.db'));
  const uuid = randomUUID();
  const requestId = randomUUID();
  try {
    db.exec('PRAGMA busy_timeout=5000');
    db.prepare("INSERT INTO dictionary(term,uuid,created_at) VALUES ('Synthetic expired policy word',?,datetime('now','-20 days'))").run(uuid);
    db.prepare('INSERT INTO dictionary_contexts(context_id,dictionary_id) SELECT id,(SELECT id FROM dictionary WHERE uuid=?) FROM contexts WHERE is_everywhere=1').run(uuid);
    db.prepare("INSERT INTO vocabulary_expiration_marks(item_uuid,cutoff,deleted_at,request_id) VALUES (?,datetime('now','-7 days'),datetime('now'),?)").run(uuid, requestId);
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.text"]').click();
    const deleted = page.locator('.setting-row').filter({ hasText: 'Deleted vocabulary' });
    await expect(deleted).toContainText('Synthetic expired policy word');
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request?.command === 'get_vocabulary_expiration') {
        await route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Synthetic status failure' }) });
      } else {
        await route.continue();
      }
    });
    await deleted.getByRole('button', { name: 'Restore', exact: true }).click();
    const row = page.locator('[data-setting-target="text-vocabulary-expiration"]');
    await expect(row.getByRole('status')).toHaveText('Unavailable');
    await expect(deleted).toHaveCount(0);
    expect((await session.invoke('get_vocabulary_expiration')).deleted.some(item => item.request_id === requestId)).toBe(false);
    await expect(page.getByRole('alert')).toContainText('Vocabulary restored, but its status could not be refreshed');
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    db.prepare('DELETE FROM vocabulary_expiration_marks WHERE item_uuid=?').run(uuid);
    db.prepare('DELETE FROM vocabulary_expiration_usage WHERE item_uuid=?').run(uuid);
    db.prepare('DELETE FROM dictionary WHERE uuid=?').run(uuid);
    db.close();
  }
});

test('a saved policy is not replaced by a stale label when refresh fails', async ({ page, session }) => {
  await session.invoke('set_vocabulary_expiration', { days: 60 });
  try {
    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.text"]').click();
    const row = page.locator('[data-setting-target="text-vocabulary-expiration"]');
    const select = row.getByRole('button', { name: 'Delete unused vocabulary after' });
    await expect(select).toHaveText('60 days');
    await page.route('**/__verenu_dev/invoke', async route => {
      const request = route.request().postDataJSON();
      if (request?.command === 'get_vocabulary_expiration') {
        await route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Synthetic status failure' }) });
      } else {
        await route.continue();
      }
    });
    await select.click();
    await page.getByRole('option', { name: '7 days', exact: true }).click();
    await expect(row.getByRole('status')).toHaveText('Unavailable');
    await expect(select).toHaveCount(0);
    expect((await session.invoke('get_vocabulary_expiration')).days).toBe(7);
    await expect(page.getByRole('alert')).toContainText('Policy saved, but its status could not be refreshed');
  } finally {
    await page.unrouteAll({ behavior: 'wait' });
    await session.invoke('set_vocabulary_expiration', { days: null });
  }
});
