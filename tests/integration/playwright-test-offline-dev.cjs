'use strict';

const { chromium } = require('playwright');
const { TARGET_URL, TIMEOUT, seedDevState } = require('./_dev-helpers.cjs');

(async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    for (const [platform, userAgent, animation] of [
      ['Linux', 'Mozilla/5.0 (X11; Linux x86_64)', 'none'],
      ['Windows', 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)', 'dot-pulse'],
      ['macOS', 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', 'dot-pulse'],
    ]) {
      const page = await browser.newPage({ userAgent });
      await seedDevState(page, {
        settings: { setup_complete: true, force_setup_on_launch: false },
      });
      await page.addInitScript((platformName) => {
        Object.defineProperty(window.navigator, 'onLine', { configurable: true, get: () => false });
        Object.defineProperty(window.navigator, 'userAgentData', {
          configurable: true, value: { platform: platformName },
        });
      }, platform);
      await page.goto(TARGET_URL, { waitUntil: 'networkidle', timeout: TIMEOUT });
      const toast = page.locator('.offline-toast:has-text("No internet connection")');
      await toast.waitFor({ state: 'visible', timeout: TIMEOUT });
      const actual = await toast.locator('.offline-dot').evaluate((el) => getComputedStyle(el).animationName);
      if (animation === 'none' ? actual !== 'none' : !actual.includes(animation)) {
        throw new Error(`${platform} offline indicator animation was ${actual}, expected ${animation}`);
      }
      await page.close();
      console.log(`PASS - ${platform} offline status is visible with the expected animation.`);
    }
  } catch (err) {
    console.error(`FAIL - offline toast test threw: ${err.message}`);
    process.exit(1);
  } finally {
    await browser.close();
  }
})();
