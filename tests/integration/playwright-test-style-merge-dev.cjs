'use strict';

const { chromium } = require('playwright');
const { TARGET_URL, TIMEOUT, seedDevState } = require('./_dev-helpers.cjs');

(async () => {
  const browser = await chromium.launch({ headless: true });
  const errors = [];

  async function newPage(settings) {
    const page = await browser.newPage();
    await seedDevState(page, { settings });
    page.on('pageerror', (err) => errors.push(`Page exception: ${err.message}`));
    page.on('console', (msg) => {
      if (msg.type() === 'error') errors.push(`Console error: ${msg.text()}`);
    });
    await page.goto(TARGET_URL, { waitUntil: 'networkidle', timeout: TIMEOUT });
    await page.locator('h1.page-h:has-text("Welcome back")').waitFor({ state: 'visible', timeout: TIMEOUT });
    await page.locator('.nav-item:has-text("Style")').click();
    await page.locator('h1.page-h:has-text("Style")').waitFor({ state: 'visible', timeout: TIMEOUT });
    return page;
  }

  try {
    {
      const page = await newPage({
        setup_complete: true,
        cleanup_enabled: true,
        cleanup_intensity: 'medium',
        default_tone: 'casual',
        legacy_features_enabled: false,
      });

      if (await page.locator('[role="tablist"]').count()) {
        errors.push('Merged Style page still rendered a tablist');
      }
      await page.locator('h2.style-section-h:has-text("Cleanup")').waitFor({ state: 'visible', timeout: TIMEOUT });
      await page.locator('h2.style-section-h:has-text("Personal Tone")').waitFor({ state: 'visible', timeout: TIMEOUT });

      await page.locator('.style-card:has-text("Strong")').click();
      await page.locator('.style-card:has-text("Formal")').click();

      const persisted = await page.evaluate(() => JSON.parse(localStorage.getItem('verenu:dev-settings') || '{}'));
      if (persisted.cleanup_intensity !== 'high') errors.push('Merged Style page did not persist cleanup intensity');
      if (persisted.default_tone !== 'formal') errors.push('Merged Style page did not persist tone');

      await page.close();
    }

    {
      const page = await newPage({
        setup_complete: true,
        cleanup_enabled: true,
        cleanup_intensity: 'medium',
        default_tone: 'casual',
        legacy_features_enabled: true,
      });

      await page.locator('[role="tablist"]').waitFor({ state: 'visible', timeout: TIMEOUT });
      if (await page.locator('h2.style-section-h').count()) {
        errors.push('Legacy Style page unexpectedly rendered the merged section headings');
      }

      await page.locator('.tab:has-text("Cleanup")').waitFor({ state: 'visible', timeout: TIMEOUT });
      await page.locator('.style-card:has-text("Strong")').click();
      await page.locator('.tab:has-text("Personal Tone")').click();
      await page.locator('.style-card:has-text("Formal")').click();

      const persisted = await page.evaluate(() => JSON.parse(localStorage.getItem('verenu:dev-settings') || '{}'));
      if (persisted.cleanup_intensity !== 'high') errors.push('Legacy Style tabs did not persist cleanup intensity');
      if (persisted.default_tone !== 'formal') errors.push('Legacy Style tabs did not persist tone');

      await page.close();
    }

    {
      const page = await newPage({ setup_complete: true, cleanup_enabled: true });
      const navSvgPaths = await page.locator('.nav-item:has-text("Style") svg path').evaluateAll((els) => els.map((el) => el.getAttribute('d')));
      if (!navSvgPaths.some((d) => d?.includes('16.5 3.5'))) {
        errors.push(`Style nav icon does not look like the pencil glyph (paths=${JSON.stringify(navSvgPaths)})`);
      }
      await page.close();
    }

    if (errors.length > 0) {
      console.error('FAIL');
      for (const error of errors) console.error(`  ${error}`);
      process.exit(1);
    }

    console.log('PASS - Style page merges Cleanup/Personal Tone by default and reverts to tabs under Legacy features.');
  } catch (err) {
    console.error(`FAIL - style merge test threw: ${err.message}`);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
