import { test, expect } from './fixtures.mjs';
import fs from 'node:fs/promises';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

test('saved vocabulary can be restored, retained, and explicitly discarded', async ({ page }, testInfo) => {
  if (testInfo.project.name === 'phone') {
    await page.addInitScript(() => { window.__VERENU_ANDROID__ = true; });
    await page.reload();
    await expect(page.getByRole('button', { name: /^Dev tests/ })).toBeVisible();
  }
  const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
  const manifest = JSON.parse(await fs.readFile(path.join(directory, 'session.json'), 'utf8'));
  expect(manifest.syntheticSeed, 'Only seed an owned public fixture database').toBe(true);
  const db = new DatabaseSync(path.join(directory, 'data', 'verenu.db'));
  const id = testInfo.project.name.includes('phone') ? 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2' : 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1';
  const term = `Synthetic review ${testInfo.project.name}`;
  const context = db.prepare('SELECT uuid FROM contexts WHERE is_everywhere=1').get().uuid;
  const payload = JSON.stringify({ term, auto_learned: false, correction_count: 0, confidence_tier: 'low', last_seen_at: null, created_at: '2020-01-01 00:00:00', replacement_confidence: 'high' });
  try {
    db.exec('PRAGMA busy_timeout=5000');
    db.prepare("INSERT INTO vocabulary_membership_reviews(context_uuid,dictionary_uuid,dictionary_term,reason) VALUES (?,?,?,'offline_deletion')").run(context,id,term);
    db.prepare("INSERT INTO vocabulary_review_payloads(kind,row_uuid,dictionary_uuid,payload,ts_ms,origin,origin_seq) VALUES ('dictionary',?,?,?,1,'synthetic-review',1)").run(id,id,payload);
    const skipLegal = page.getByRole('button', { name: 'Skip for now', exact: true });
    await skipLegal.waitFor({ state: 'visible', timeout: 5000 }).catch(() => {});
    if (await skipLegal.isVisible()) await skipLegal.click();
    if (testInfo.project.name === 'phone') {
      await page.locator('.mobile-nav').getByRole('button', { name: 'Settings', exact: true }).click();
      await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Text', exact: true }).click();
    } else {
      await page.locator('[data-debug-id="nav.settings"]').click();
      await page.locator('[data-debug-id="settings.text"]').click();
    }
    await page.getByRole('button', { name: /^Dev tests/ }).click();
    const devPanel = page.getByRole('complementary', { name: 'Dev session tests' });
    await expect(devPanel).toBeVisible();
    const bounds = await devPanel.boundingBox();
    expect(bounds.y).toBeGreaterThanOrEqual(0);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(testInfo.project.use.viewport.height);
    await devPanel.getByRole('button', { name: 'Close', exact: true }).click();
    const section = page.locator('.setting-row').filter({ hasText: 'Vocabulary to review' });
    await expect(section).toContainText(term);
    await expect(section).toContainText('confidence high');
    await section.scrollIntoViewIfNeeded();
    await expect.poll(() => section.evaluate(element => {
      let opacity = 1;
      for (let node = element; node; node = node.parentElement) opacity *= Number(getComputedStyle(node).opacity);
      return opacity;
    })).toBeGreaterThan(0.99);
    await page.screenshot({ path: testInfo.outputPath('vocabulary-review-before.png') });
    await section.getByRole('button', { name: 'Restore', exact: true }).click();
    await expect.poll(() => db.prepare('SELECT COUNT(*) AS n FROM dictionary WHERE uuid=?').get(id).n).toBe(1);
    await expect(section).toContainText('Saved vocabulary');
    await expect(page.getByRole('status').filter({ hasText: 'Saved change restored' })).toBeVisible();
    const discard = section.getByRole('button', { name: 'Discard', exact: true });
    await discard.evaluate(button => button.scrollIntoView({ block: 'center', behavior: 'instant' }));
    await page.screenshot({ path: testInfo.outputPath('vocabulary-review-restored.png') });
    expect(await discard.evaluate(button => {
      const box = button.getBoundingClientRect();
      const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
      return hit === button || button.contains(hit);
    }), 'The saved-change action must not be covered by the dev-session launcher').toBe(true);
    page.once('dialog', dialog => dialog.accept());
    await discard.click();
    await expect.poll(() => db.prepare('SELECT COUNT(*) AS n FROM vocabulary_review_payloads WHERE row_uuid=?').get(id).n).toBe(0);
    await section.getByRole('button', { name: 'Mark reviewed', exact: true }).click();
    await expect(section).toHaveCount(0);
    expect(db.prepare('SELECT COUNT(*) AS n FROM dictionary WHERE uuid=?').get(id).n).toBe(1);
  } finally {
    db.prepare('DELETE FROM vocabulary_membership_reviews WHERE dictionary_uuid=?').run(id);
    db.prepare('DELETE FROM vocabulary_review_payloads WHERE dictionary_uuid=?').run(id);
    db.prepare('DELETE FROM dictionary WHERE uuid=?').run(id);
    db.close();
  }
});

test('the dev launcher leaves Android Settings navigation usable', async ({ browser, session }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  await context.addInitScript(() => { window.__VERENU_ANDROID__ = true; });
  const page = await context.newPage();
  try {
    await page.goto(session.access.localAccessUrl);
    const skipLegal = page.getByRole('button', { name: 'Skip for now', exact: true });
    await skipLegal.waitFor({ state: 'visible', timeout: 5000 }).catch(() => {});
    if (await skipLegal.isVisible()) await skipLegal.click();
    const settings = page.locator('.mobile-nav').getByRole('button', { name: 'Settings', exact: true });
    await settings.click();
    await expect(page.getByRole('navigation', { name: 'Settings sections' })).toBeVisible();
    expect(await settings.evaluate(button => {
      const box = button.getBoundingClientRect();
      const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
      return hit === button || button.contains(hit);
    }), 'The Settings navigation button must not be covered by the dev-session launcher').toBe(true);
  } finally {
    await context.close();
  }
});
