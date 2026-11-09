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

test('Home stats stay visible while history scrolls and remain hidden on narrow windows', async ({ page }, testInfo) => {
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
    const content = page.locator('.content');
    if (testInfo.project.name === 'desktop') {
      const initial = await stats.boundingBox();
      await page.screenshot({ path: testInfo.outputPath('home-top.png') });
      for (const scrollTop of [650, Number.MAX_SAFE_INTEGER, 0]) {
        await content.evaluate((element, top) => { element.scrollTop = top; }, scrollTop);
        await expect.poll(async () => (await stats.boundingBox()).y).toBeCloseTo(initial.y, 0);
        const bounds = await stats.boundingBox();
        const viewport = await content.boundingBox();
        expect(bounds.y).toBeGreaterThanOrEqual(viewport.y);
        expect(bounds.y + bounds.height).toBeLessThan(viewport.y + viewport.height);
        if (scrollTop > 0) expect(await content.evaluate(element => element.scrollTop)).toBeGreaterThan(300);
      }
      await content.evaluate(element => { element.scrollTop = 650; });
      await page.screenshot({ path: testInfo.outputPath('home-scrolled.png') });
      await page.setViewportSize({ width: 1000, height: 800 });
      await expect(stats).toBeHidden();
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
