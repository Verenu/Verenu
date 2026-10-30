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
  const dayWords = db.prepare("SELECT SUM(words) AS words FROM transcriptions WHERE date(created_at, 'localtime') = date('now', 'localtime')").get().words;
  try {
    await page.reload();
    await page.locator('[data-debug-id="nav.insights"]').click();
    const plot = page.locator('.plot').first();
    await expect(plot).toBeVisible();
    await expect(plot).toHaveRole('slider');
    await plot.focus();
    await page.keyboard.press('Home');
    await expect(plot).toHaveAttribute('aria-valuenow', '0');
    await page.keyboard.press('ArrowRight');
    await expect(plot).toHaveAttribute('aria-valuenow', '1');
    await page.keyboard.press('ArrowUp');
    await expect(plot).toHaveAttribute('aria-valuenow', '2');
    await page.keyboard.press('ArrowDown');
    await expect(plot).toHaveAttribute('aria-valuenow', '1');
    await page.keyboard.press('ArrowLeft');
    await expect(plot).toHaveAttribute('aria-valuenow', '0');
    await page.keyboard.press('End');
    await expect(plot).toHaveAttribute('aria-valuenow', await plot.getAttribute('aria-valuemax'));
    await expect(plot).toHaveAttribute('aria-valuetext', new RegExp(`${new Intl.NumberFormat('en-US').format(dayWords)} words`));
    await plot.blur();
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
  await expect(page.getByLabel('Chart scale details')).toHaveCount(0);
  expect(await session.invoke('get_setting', { key: 'github_username' })).toBe('');
  expect(await session.invoke('get_github_commits')).toBeNull();
  await expect(page.locator('.github-line')).toHaveCount(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('connected GitHub activity stays in the chart without a status row and retains warnings', async ({ page, session }) => {
  const previousUsername = await session.invoke('get_setting', { key: 'github_username' });
  let fixtureUsername = 'octocat';
  const dayKey = (date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  const snapshot = {
    source: 'contributions',
    username: fixtureUsername,
    fetched_at: Math.floor(Date.now() / 1000),
    start_day: dayKey(yesterday),
    end_day: dayKey(today),
    utc_offset: -today.getTimezoneOffset() * 60,
    complete: true,
    daily: [{ day: dayKey(yesterday), commits: 2 }, { day: dayKey(today), commits: 3 }],
    warning: 'GitHub activity may be incomplete.',
  };
  await page.route('**/__verenu_dev/invoke', async (route) => {
    if (route.request().postDataJSON()?.command === 'get_github_commits') {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ...snapshot, username: fixtureUsername }) });
    } else {
      await route.continue();
    }
  });
  try {
    await session.invoke('save_setting', { key: 'github_username', value: 'octocat' });
    await page.reload();
    await page.locator('[data-debug-id="nav.insights"]').click();

    const chart = page.getByRole('heading', { name: 'Words per day', exact: true });
    await expect(chart).toBeVisible();
    await expect(page.locator('.chart-legend')).toContainText(/Contributions|Public commits/);
    await expect(page.locator('.github-line')).toHaveCount(1);
    await expect(page.locator('.github-warning')).toContainText('GitHub activity may be incomplete.');
    await expect(page.locator('.github-status')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Manage in Settings', exact: true })).toHaveCount(0);

    await page.locator('[data-debug-id="nav.settings"]').click();
    await page.locator('[data-debug-id="settings.integrations"]').click();
    await expect(page.getByText('@octocat', { exact: true })).toBeVisible();
  } finally {
    fixtureUsername = previousUsername ?? '';
    await session.invoke('save_setting', { key: 'github_username', value: previousUsername ?? '' });
    await page.reload();
  }
});
