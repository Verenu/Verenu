import fs from 'node:fs/promises';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { test, expect } from './fixtures.mjs';

test('history renders while its independent stats request is still pending', async ({ page, session }) => {
  const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
  const db = new DatabaseSync(path.join(directory, 'data', 'verenu.db'));
  let row;
  let releaseStats;
  const statsHeld = new Promise(resolve => { releaseStats = resolve; });
  let statsRequested;
  const requested = new Promise(resolve => { statsRequested = resolve; });
  try {
    db.exec('PRAGMA busy_timeout=5000');
    const text = 'Synthetic history loads before statistics';
    row = db.prepare('INSERT INTO transcriptions(raw_text,clean_text,words,duration_ms,api_used) VALUES(?,?,1,1000,?) RETURNING id').get(text, text, 'fixture');
    const expectedStats = await session.invoke('get_stats');
    const expectedWords = expectedStats.total_words >= 1000
      ? `${(expectedStats.total_words / 1000).toFixed(1)}k`
      : String(expectedStats.total_words);
    await page.route('**/__verenu_dev/invoke', async route => {
      if (route.request().postDataJSON()?.command !== 'get_stats') return route.continue();
      const response = await route.fetch();
      statsRequested();
      await statsHeld;
      await route.fulfill({ response });
    });
    await page.reload({ waitUntil: 'domcontentloaded' });
    await requested;
    await expect(page.getByText(text, { exact: true })).toBeVisible();
    releaseStats();
    await expect(page.locator('.stat-card .stat-num').first()).toHaveText(expectedWords);
  } finally {
    releaseStats();
    try {
      await page.unroute('**/__verenu_dev/invoke');
    } finally {
      try {
        if (row) db.prepare('DELETE FROM transcriptions WHERE id=?').run(row.id);
      } finally {
        db.close();
      }
    }
  }
});

test('Home stats sit in the page header on desktop and stay hidden on phone widths', async ({ page }, testInfo) => {
  const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
  const manifest = JSON.parse(await fs.readFile(path.join(directory, 'session.json'), 'utf8'));
  expect(manifest.syntheticSeed, 'Use disposable public fixture data').toBe(true);
  const db = new DatabaseSync(path.join(directory, 'data', 'verenu.db'));
  db.exec('PRAGMA busy_timeout=5000');
  const insert = db.prepare('INSERT INTO transcriptions(raw_text,clean_text,words,spoken_words,duration_ms,api_used,app_name) VALUES(?,?,10,10,4000,?,?) RETURNING id');
  const ids = [];
  try {
    for (let i = 1; i <= 60; i++) {
      const text = `Synthetic scrolling history ${i}. This public sample checks the Home layout.`;
      ids.push(insert.get(text, text, 'fixture-model', 'Synthetic app').id);
    }
    await page.reload();
    await expect(page.locator('.day-row').first()).toBeVisible();
    const stats = page.locator('.stat-stack');
    if (testInfo.project.name === 'desktop') {
      const expectInHeader = async () => {
        await expect(stats).toBeVisible();
        const head = await page.locator('.home-head').boundingBox();
        const title = await page.locator('h1.page-h').boundingBox();
        const bounds = await stats.boundingBox();
        const firstRow = await page.locator('.day-row').first().boundingBox();
        expect(bounds.x).toBeGreaterThan(title.x + title.width);
        expect(bounds.y).toBeGreaterThanOrEqual(head.y);
        expect(bounds.y + bounds.height).toBeLessThanOrEqual(head.y + head.height + 0.5);
        expect(Math.abs(bounds.x + bounds.width - (firstRow.x + firstRow.width))).toBeLessThan(1);
        const numbers = await stats.locator('.stat-num').evaluateAll(nodes => nodes.map(node => node.getBoundingClientRect().y));
        expect(numbers).toHaveLength(3);
        expect(Math.max(...numbers) - Math.min(...numbers)).toBeLessThan(1);
      };
      await expectInHeader();
      await page.screenshot({ path: testInfo.outputPath('home-top.png') });
      await page.setViewportSize({ width: 1000, height: 800 });
      await expectInHeader();
      await page.screenshot({ path: testInfo.outputPath('home-narrow.png') });
    } else {
      await expect(stats).toBeHidden();
      await page.screenshot({ path: testInfo.outputPath('home-phone.png') });
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  } finally {
    const remove = db.prepare('DELETE FROM transcriptions WHERE id=?');
    for (const id of ids) remove.run(id);
    db.close();
  }
});

test('Home hero is a slim reminder once someone has dictated and a full card until then', async ({ page, readySpeech }) => {
  const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
  const db = new DatabaseSync(path.join(directory, 'data', 'verenu.db'));
  db.exec('PRAGMA busy_timeout=5000');
  const hero = page.locator('.hero-photo');
  let row;
  try {
    await expect(page.locator('.empty-state')).toContainText('No dictations yet');
    await expect(hero.locator('.hero-photo-sub')).toContainText('Verenu works in any app');
    await expect(hero).not.toHaveClass(/compact/);
    const full = (await hero.boundingBox()).height;

    const text = 'Synthetic returning user sample';
    row = db.prepare('INSERT INTO transcriptions(raw_text,clean_text,words,duration_ms,api_used) VALUES(?,?,4,1000,?) RETURNING id').get(text, text, 'fixture');
    await page.reload();
    await expect(page.getByText(text, { exact: true })).toBeVisible();
    await expect(hero).toHaveClass(/compact/);
    await expect(hero.locator('.hero-photo-title')).toContainText('to dictate');
    await expect(hero.locator('.hero-photo-sub')).toHaveCount(0);
    const slim = (await hero.boundingBox()).height;
    expect(slim).toBeLessThan(full);
    expect(slim).toBeLessThan(60);

    // The slim state is remembered, so the next load starts slim instead of collapsing after history arrives.
    await page.reload();
    await expect(hero).toBeVisible();
    await expect(hero).toHaveClass(/compact/);
  } finally {
    if (row) db.prepare('DELETE FROM transcriptions WHERE id=?').run(row.id);
    db.close();
  }
});

test('Home hero keeps the full setup card while dictation setup is incomplete', async ({ page, incompleteSpeech }) => {
  const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
  const db = new DatabaseSync(path.join(directory, 'data', 'verenu.db'));
  db.exec('PRAGMA busy_timeout=5000');
  let row;
  try {
    const text = 'Synthetic incomplete setup sample';
    row = db.prepare('INSERT INTO transcriptions(raw_text,clean_text,words,duration_ms,api_used) VALUES(?,?,4,1000,?) RETURNING id').get(text, text, 'fixture');
    await page.reload();
    await expect(page.getByText(text, { exact: true })).toBeVisible();
    const hero = page.locator('.hero-photo');
    await expect(hero.locator('.hero-photo-title')).toHaveText('Finish dictation setup');
    await expect(hero.locator('.hero-photo-sub')).toBeVisible();
    await expect(hero).not.toHaveClass(/compact/);
  } finally {
    if (row) db.prepare('DELETE FROM transcriptions WHERE id=?').run(row.id);
    db.close();
  }
});
