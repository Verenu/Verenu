import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { test, expect } from './fixtures.mjs';

for (const preference of ['no-preference', 'reduce']) {
  test(`Context dialog motion settles and remains operable with ${preference}`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: preference });
    await page.evaluate(() => {
      const animate = Element.prototype.animate;
      window.__dialogMotion = [];
      const capture = (element, animation) => {
        if (element.closest('[role="dialog"]')) window.__dialogMotion.push(animation.effect.getTiming().duration);
      };
      Element.prototype.animate = function (...args) {
        const animation = animate.apply(this, args);
        capture(this, animation);
        return animation;
      };
      for (const event of ['animationstart', 'transitionrun']) {
        document.addEventListener(event, ({ target }) => {
          for (const animation of target.getAnimations()) capture(target, animation);
        }, true);
      }
    });
    await page.getByRole('button', { name: 'New context group', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'New context group', exact: true });
    await expect(dialog).toBeVisible();
    await expect.poll(() => dialog.evaluate(element => element.getAnimations({ subtree: true }).filter(animation => animation.playState === 'running').length)).toBe(0);
    const motion = await page.evaluate(() => window.__dialogMotion);
    expect(motion.length).toBeGreaterThan(0);
    if (preference === 'no-preference') expect(motion.some(duration => duration > 0)).toBe(true);
    else expect(motion.every(duration => duration <= 132)).toBe(true);
    await dialog.getByLabel('Context group name').fill('Motion regression');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByRole('button', { name: 'New context group', exact: true })).toBeEnabled();
  });
}

// Records, once per frame, the things that used to jump: window scroll, the rail lists, and the history search field.
async function watchFrames(page) {
  await page.evaluate(() => {
    const frames = [];
    let running = true;
    const box = (selector) => {
      const element = document.querySelector(selector);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { y: rect.y, left: rect.left, width: rect.width };
    };
    const tick = () => {
      if (!running) return;
      frames.push({
        scroll: Math.max(window.scrollY, document.body.scrollTop, document.documentElement.scrollTop),
        appRail: box('.rail-list-app'),
        settingsRail: box('.rail-list-settings'),
        search: box('.history-search-group'),
        notices: document.querySelectorAll('.readiness-notice, .loading-state').length,
        rows: document.querySelectorAll('.page-wrapper[data-page="home"] .day-row').length,
        hero: box('.page-wrapper[data-page="home"] .hero-photo'),
      });
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    window.__frames = { stop() { running = false; return frames; } };
  });
  return () => page.evaluate(() => window.__frames.stop());
}

const spread = (values) => (values.length ? Math.max(...values) - Math.min(...values) : 0);

const navButton = (page, id) => page.locator(`[data-debug-id="nav.${id}"]`);

test('opening and closing Settings never scrolls the window or shifts the rail lists', async ({ page }) => {
  await expect(page.locator('.rail-list-app')).toBeVisible();

  let stop = await watchFrames(page);
  await navButton(page, 'settings').click();
  await expect(page.locator('.settings-page')).toBeVisible();
  await expect(page.locator('.rail-list-settings')).toBeVisible();
  await expect(page.locator('.rail-list-app')).toHaveCount(0);
  await page.waitForTimeout(400);
  let frames = await stop();
  expect(frames.length).toBeGreaterThan(5);
  expect(Math.max(...frames.map(frame => frame.scroll))).toBe(0);
  expect(spread(frames.filter(frame => frame.appRail).map(frame => frame.appRail.y))).toBeLessThan(1);
  expect(spread(frames.filter(frame => frame.appRail).map(frame => frame.appRail.left))).toBeLessThan(1);
  expect(spread(frames.filter(frame => frame.settingsRail).map(frame => frame.settingsRail.y))).toBeLessThan(1);

  stop = await watchFrames(page);
  await page.keyboard.press('Escape');
  await expect(page.locator('.rail-list-settings')).toHaveCount(0);
  frames = await stop();
  expect(Math.max(...frames.map(frame => frame.scroll))).toBe(0);
  expect(spread(frames.filter(frame => frame.settingsRail).map(frame => frame.settingsRail.y))).toBeLessThan(1);
  expect(spread(frames.filter(frame => frame.settingsRail).map(frame => frame.settingsRail.left))).toBeLessThan(1);
  expect(spread(frames.filter(frame => frame.appRail).map(frame => frame.appRail.y))).toBeLessThan(1);
});

test('returning to Home repaints the previous rows without a loading state or a hero change', async ({ page, readySpeech }) => {
  const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
  const db = new DatabaseSync(path.join(directory, 'data', 'verenu.db'));
  db.exec('PRAGMA busy_timeout=5000');
  const text = 'Synthetic row that stays painted across navigation';
  const row = db.prepare('INSERT INTO transcriptions(raw_text,clean_text,words,spoken_words,duration_ms,api_used) VALUES(?,?,8,8,3000,?) RETURNING id').get(text, text, 'fixture');
  try {
    await page.reload({ waitUntil: 'domcontentloaded' });
    await expect(page.getByText(text, { exact: true })).toBeVisible();
    await expect(page.locator('.hero-photo.compact')).toBeVisible();
    await expect(page.locator('.stat-stack')).not.toHaveClass(/pending/);

    await navButton(page, 'insights').click();
    await expect(page.locator('.page-wrapper[data-page="insights"]')).toBeVisible({ timeout: 30_000 });
    await expect(page.locator('.page-wrapper')).toHaveCount(1);

    const stop = await watchFrames(page);
    await navButton(page, 'home').click();
    await expect(page.locator('.page-wrapper[data-page="home"]')).toBeVisible();
    await expect(page.locator('.page-wrapper')).toHaveCount(1);
    await expect(page.getByText(text, { exact: true })).toBeVisible();
    await page.waitForTimeout(600);
    const frames = (await stop()).filter(frame => frame.hero);
    expect(frames.length).toBeGreaterThan(5);
    expect(Math.max(...frames.map(frame => frame.notices))).toBe(0);
    expect(Math.min(...frames.map(frame => frame.rows))).toBeGreaterThan(0);
    expect(spread(frames.map(frame => frame.hero.width))).toBeLessThan(1);
    await expect(page.locator('.hero-photo.compact')).toBeVisible();
  } finally {
    db.prepare('DELETE FROM transcriptions WHERE id=?').run(row.id);
    db.close();
  }
});

test('the history search field grows and shrinks without snapping past its open width', async ({ page }) => {
  const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
  const db = new DatabaseSync(path.join(directory, 'data', 'verenu.db'));
  db.exec('PRAGMA busy_timeout=5000');
  const text = 'Synthetic row for the search field motion';
  const row = db.prepare('INSERT INTO transcriptions(raw_text,clean_text,words,spoken_words,duration_ms,api_used) VALUES(?,?,8,8,3000,?) RETURNING id').get(text, text, 'fixture');
  try {
    await page.reload({ waitUntil: 'domcontentloaded' });
    await expect(page.getByText(text, { exact: true })).toBeVisible();
    const group = page.locator('.history-search-group');
    const stop = await watchFrames(page);
    await page.getByRole('button', { name: 'Search history', exact: true }).click();
    await expect(group).toHaveClass(/settled/);
    const open = (await group.boundingBox()).width;
    expect(open).toBeGreaterThan(200);
    await page.locator('h1.page-h').click();
    await expect(group).not.toHaveClass(/expanded/);
    await expect.poll(async () => (await group.boundingBox()).width).toBeLessThan(40);
    const widths = (await stop()).filter(frame => frame.search).map(frame => frame.search.width);
    expect(Math.max(...widths)).toBeLessThanOrEqual(open + 1);
    const intermediate = widths.filter(width => width > 40 && width < open - 8);
    expect(intermediate.length, 'the width animates through intermediate values').toBeGreaterThan(1);
  } finally {
    db.prepare('DELETE FROM transcriptions WHERE id=?').run(row.id);
    db.close();
  }
});
