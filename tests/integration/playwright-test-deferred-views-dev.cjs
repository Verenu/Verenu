'use strict';

const assert = require('node:assert/strict');
const { chromium } = require('playwright');
const { TARGET_URL, TIMEOUT, seedDevState, openSettings } = require('./_dev-helpers.cjs');

(async () => {
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const requests = [];
  const errors = [];
  page.on('request', (request) => requests.push(request.url()));
  page.on('pageerror', (error) => errors.push(error.message));
  await seedDevState(page, { settings: { setup_complete: true, dev_mode_on_startup: true } });
  await page.route('**/AudioSection.svelte', async (route) => {
    // Exceeds the old search highlight's 40-frame polling window.
    await new Promise((resolve) => setTimeout(resolve, 1100));
    await route.continue();
  });
  let developerAttempts = 0;
  await page.route('**/DeveloperSection.svelte', async (route) => {
    if (++developerAttempts === 1) await route.abort('failed');
    else await route.continue();
  });

  try {
    await page.goto(TARGET_URL, { waitUntil: 'networkidle', timeout: TIMEOUT });
    await page.locator('.nav-item').first().waitFor({ state: 'visible', timeout: TIMEOUT });
    assert(!requests.some((url) => /\/(Setup|ModelsSection|DeveloperSection|Insights)\.svelte/.test(url)),
      'unused features must not load on Home');
    await openSettings(page);
    await page.locator('h2.settings-h', { hasText: 'General' }).waitFor({ state: 'visible', timeout: TIMEOUT });
    await page.getByRole('searchbox', { name: 'Search settings' }).fill('noise reduction');
    await page.locator('.settings-search-result', { hasText: 'Noise reduction' }).click();
    const target = page.locator('[data-setting-target="audio-noise"]');
    await target.waitFor({ state: 'visible', timeout: TIMEOUT });
    await page.waitForFunction(() => document.activeElement?.matches('[data-setting-target="audio-noise"]'), null, { timeout: TIMEOUT });
    assert(await target.evaluate((element) => element.classList.contains('settings-search-hit')),
      'search must wait for the deferred section before highlighting');

    await page.locator('.settings-nav-item', { hasText: 'Developer' }).click();
    await page.getByRole('alert').filter({ hasText: "Couldn't load this view." }).waitFor({ state: 'visible', timeout: TIMEOUT });
    // Browsers can cache a failed module fetch for the life of a document.
    // Reload clears that failure while saved settings remain in storage.
    await Promise.all([
      page.waitForEvent('load'),
      page.getByRole('button', { name: 'Reload app', exact: true }).click(),
    ]);
    await openSettings(page);
    await page.locator('.settings-nav-item', { hasText: 'Developer' }).click();
    await page.locator('h2.settings-h', { hasText: 'Developer' }).waitFor({ state: 'visible', timeout: TIMEOUT });
    assert.equal(developerAttempts, 2, 'retry must fetch the failed module again');
    assert.deepEqual(errors, [], 'deferred loading must not cause uncaught errors');

    const phone = await browser.newPage({
      viewport: { width: 390, height: 844 },
      userAgent: 'Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 Chrome/120.0 Mobile Safari/537.36',
    });
    const phoneRequests = [];
    phone.on('request', (request) => phoneRequests.push(request.url()));
    phone.on('pageerror', (error) => errors.push(error.message));
    await seedDevState(phone, { settings: { setup_complete: true } });
    await phone.goto(TARGET_URL, { waitUntil: 'networkidle', timeout: TIMEOUT });
    assert(!phoneRequests.some((url) => url.includes('/PermissionsSection.svelte')),
      'Android permissions must remain deferred on Home');
    await phone.locator('.mobile-nav').getByRole('button', { name: 'Settings', exact: true }).click();
    await phone.getByRole('tab', { name: 'Permissions', exact: true }).click();
    await phone.locator('h2.settings-h', { hasText: 'Permissions' }).waitFor({ state: 'visible', timeout: TIMEOUT });
    await phone.locator('.android-perms').waitFor({ state: 'visible', timeout: TIMEOUT });
    assert.equal(await phone.locator('.android-perms .perm-row').count(), 4,
      'the deferred Android section must render its permission controls');
    assert.deepEqual(errors, [], 'Android deferred loading must not cause uncaught errors');
    await phone.close();
    console.log('PASS - deferred features, slow search navigation, and load retry verified.');
  } catch (error) {
    console.error(`FAIL - deferred views: ${error.message}`);
    process.exitCode = 1;
  } finally {
    await browser.close();
  }
})();
