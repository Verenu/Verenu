import fs from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import { test, expect } from './fixtures.mjs';

test('open General settings reflects a shared setting after sync without changing appearance', async ({ page, session }) => {
  const initial = await session.invoke('get_setting', { key: 'contextual_formatting_enabled' });
  const appearance = await session.invoke('get_setting', { key: 'appearance_mode' });
  try {
    await session.invoke('save_setting', { key: 'contextual_formatting_enabled', value: false });
    await page.locator('[data-debug-id="nav.settings"]').click();
    const control = page.getByRole('switch', { name: /Smart spacing/ });
    await expect(control).toHaveAttribute('aria-checked', 'false');
    await session.invoke('save_setting', { key: 'contextual_formatting_enabled', value: true });
    await page.evaluate(async () => {
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('verenu:sync-status', { state: 'synced' });
    });
    await expect(control).toHaveAttribute('aria-checked', 'true');
    expect(await session.invoke('get_setting', { key: 'appearance_mode' })).toEqual(appearance);
  } finally {
    await session.invoke('save_setting', { key: 'contextual_formatting_enabled', value: initial ?? true });
  }
});

test('Home refreshes imported history and totals when sync completes', async ({ page, session }) => {
  const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
  const manifest = JSON.parse(await fs.readFile(path.join(directory, 'session.json'), 'utf8'));
  expect(manifest.syntheticSeed, 'Use disposable public fixture data').toBe(true);
  await page.locator('[data-debug-id="nav.home"]').click();
  const before = await session.invoke('get_stats');
  await expect(page.locator('.stat-card .stat-num').first()).toHaveText(String(before.total_words));
  const text = `Synthetic synced history ${randomUUID()}`;
  const db = new DatabaseSync(path.join(directory, 'data', 'verenu.db'));
  db.exec('PRAGMA busy_timeout=5000; BEGIN IMMEDIATE;');
  let row;
  try {
    row = db.prepare('INSERT INTO transcriptions(raw_text,clean_text,words,spoken_words,duration_ms,api_used,app_name) VALUES(?,?,3,3,1200,?,?) RETURNING id').get(text, text, 'fixture-model', 'Synthetic sync app');
    db.prepare('INSERT INTO lifetime_stats(id,total_words) VALUES(1,3) ON CONFLICT(id) DO UPDATE SET total_words=total_words+3').run();
    db.exec('COMMIT;');
    // SQLite fixtures represent committed remote imports. No provider calls
    // or IPC responses are mocked; the page queries its real Rust backend.
    await page.evaluate(async () => {
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('verenu:sync-status', { state: 'syncing' });
    });
    await expect(page.getByText(text, { exact: true })).toHaveCount(0);
    await page.evaluate(async () => {
      const { emit } = await import('/src/lib/tauri.ts');
      await emit('verenu:sync-status', { state: 'synced' });
    });
    await expect(page.getByText(text, { exact: true })).toBeVisible();
    await expect(page.locator('.stat-card .stat-num').first()).toHaveText(String(before.total_words + 3));
  } finally {
    if (!row) db.exec('ROLLBACK;');
    else {
      db.prepare('DELETE FROM transcriptions WHERE id=?').run(row.id);
      db.prepare('UPDATE lifetime_stats SET total_words=total_words-3 WHERE id=1').run();
    }
    db.close();
  }
});
