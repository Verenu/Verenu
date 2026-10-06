import { test, expect } from './fixtures.mjs';
import fs from 'node:fs/promises';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

test('daily counts remain inside the chart at both date edges', async ({ page, session }) => {
  const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
  const manifest = JSON.parse(await fs.readFile(path.join(directory, 'session.json'), 'utf8'));
  expect(manifest.syntheticSeed, 'Use disposable public fixture data').toBe(true);
  await session.invoke('save_setting', { key: 'github_username', value: '' });
  const db = new DatabaseSync(path.join(directory, 'data', 'verenu.db'));
  db.exec('PRAGMA busy_timeout=5000');
  const row = db.prepare('INSERT INTO transcriptions(raw_text,clean_text,words,spoken_words,duration_ms,api_used,app_name) VALUES(?,?,200,200,40000,?,?) RETURNING id')
    .get('Public synthetic chart sample.', 'Public synthetic chart sample.', 'fixture-model', 'Synthetic app');
  try {
    await page.reload();
    await page.locator('[data-debug-id="nav.insights"]').click();
    const plot = page.locator('.plot').first();
    await expect(plot).toBeVisible();
    const tooltip = plot.locator('.chart-tooltip');
    const bounds = await plot.boundingBox();
    for (const x of [1, bounds.width - 1]) {
      await plot.hover({ position: { x, y: 50 } });
      await expect(tooltip).toHaveClass(/visible/);
      await expect.poll(async () => {
        const box = await tooltip.boundingBox();
        return box.x >= bounds.x - 1 && box.x + box.width <= bounds.x + bounds.width + 1;
      }).toBe(true);
      await expect(tooltip).toContainText('words');
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  } finally {
    db.prepare('DELETE FROM transcriptions WHERE id=?').run(row.id);
    db.close();
  }
});

test('GitHub comparison is opt-in and invalid usernames never enable it', async ({ page, session }) => {
  await session.invoke('save_setting', { key: 'github_username', value: '' });
  expect(await session.invoke('get_github_commits')).toBeNull();
  await page.locator('[data-debug-id="nav.insights"]').click();
  await expect(page.locator('#github-username')).toHaveCount(0);
  await page.locator('[data-debug-id="nav.settings"]').click();
  await page.locator('[data-debug-id="settings.integrations"]').click();
  const input = page.getByRole('textbox', { name: 'GitHub username', exact: true });
  await input.fill('a repo:private');
  await page.getByRole('button', { name: 'Connect GitHub', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('GitHub username');
  expect(await session.invoke('get_setting', { key: 'github_username' })).toBe('');
  expect(await page.locator('.github-line').count()).toBe(0);
  await page.reload();
  await page.locator('[data-debug-id="nav.insights"]').click();
  await expect(page.locator('#github-username')).toHaveCount(0);
  expect(await session.invoke('get_setting', { key: 'github_username' })).toBe('');
  expect(await session.invoke('get_github_commits')).toBeNull();
  await expect(page.locator('.github-line')).toHaveCount(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
