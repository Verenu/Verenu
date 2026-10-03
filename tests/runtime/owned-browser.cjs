// Adapt frozen smoke URLs and screenshots only inside this runner's child.
// Assertions and application responses are never changed.
const { chromium } = require('playwright');
const path = require('node:path');
const fs = require('node:fs');
const launch = chromium.launch.bind(chromium);
function attach(page) {
  if (page.__verenuOwned) return;
  page.__verenuOwned = true;
  const goto = page.goto.bind(page);
  page.goto = (url, options) => {
    const testUrl = process.env.TEST_URL;
    const target = testUrl ? String(url).replace(/^http:\/\/(localhost|127\.0\.0\.1):1420(?=\/|$)/, testUrl) : url;
    return goto(target, options);
  };
  const screenshot = page.screenshot.bind(page);
  page.screenshot = (options = {}) => {
    if (!options.path) return screenshot(options);
    const directory = process.env.VERENU_TEST_ARTIFACT_DIR;
    if (!directory) return screenshot(options);
    fs.mkdirSync(directory, { recursive: true });
    return screenshot({ ...options, path: path.join(directory, path.basename(options.path)) });
  };
}
chromium.launch = async (...args) => {
  const browser = await launch(...args);
  const newContext = browser.newContext.bind(browser);
  browser.newContext = async (...options) => {
    const context = await newContext(...options);
    context.on('page', attach);
    return context;
  };
  const newPage = browser.newPage.bind(browser);
  browser.newPage = async (...options) => { const page = await newPage(...options); attach(page); return page; };
  return browser;
};
