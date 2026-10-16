'use strict';


const url = process.argv[2] || 'http://localhost:1420';
const BUDGET_MS = Number(process.env.WARMUP_TIMEOUT_MS || 90_000);

(async () => {
  let browser;
  try {
    const { chromium } = require('playwright');
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: BUDGET_MS });
    await page.locator('.app').waitFor({ state: 'attached', timeout: BUDGET_MS });
  } catch {
  } finally {
    if (browser) await browser.close().catch(() => {});
  }
  process.exit(0);
})();
