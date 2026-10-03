import { defineConfig } from '@playwright/test';
import path from 'node:path';
if (!process.env.VERENU_SESSION_ACCESS_FILE) throw new Error('Use an owned Rust-backed session');
const directory = path.dirname(process.env.VERENU_SESSION_ACCESS_FILE);
export default defineConfig({
  testDir: '.', testMatch: '*.spec.mjs', workers: 1, retries: 0, timeout: 60_000,
  outputDir: path.join(directory, 'playwright-artifacts'),
  reporter: [['list'], ['json', { outputFile: path.join(directory, 'playwright.json') }]],
  use: { trace: 'retain-on-failure', screenshot: 'only-on-failure', video: 'retain-on-failure' },
  projects: [{ name: 'desktop', use: { viewport: { width: 1320, height: 860 } } }, { name: 'phone', use: { viewport: { width: 390, height: 844 } } }],
});
