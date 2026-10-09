// Captures real Verenu screens (the Svelte UI from this checkout) for the demo
// video. The page runs on the browser preview backend (src/lib/tauri.dev.ts)
// seeded with the synthetic demo data below; nothing comes from a real install.
import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

const [baseUrl = 'http://127.0.0.1:47613', outDir = 'ui-shots'] = process.argv.slice(2);
mkdirSync(outDir, { recursive: true });

// A fixed weekday morning keeps the greeting and history timestamps stable.
const now = new Date('2026-10-06T10:42:00').getTime();
const ago = (minutes) => new Date(now - minutes * 60_000).toISOString();
const icon = (text, background, foreground = '#ffffff') =>
  `custom-icon:${JSON.stringify({ text, background, foreground, kind: /\p{Extended_Pictographic}/u.test(text) ? 'emoji' : 'letters' })}`;
const context = (id, name, fields) => ({
  id, name, is_everywhere: id === 1, icon: null, tone: null, cleanup_intensity: null, color: null,
  custom_instructions: null, contextual_formatting_disabled: false, paste_in_chunks: false,
  pinned_at: null, created_at: ago(60 * 24 * 20), updated_at: ago(60), ...fields,
});

const seed = {
  'verenu:dev-settings': {
    __provider_connected: { groq: true },
    appearance_mode: 'light',
    setup_complete: true,
  },
  'verenu:dev-contexts': [
    context(1, 'Everywhere', {}),
    context(2, 'Team chat', { icon: icon('TC', '#c44632'), tone: 'very_casual', cleanup_intensity: 'light' }),
    context(3, 'Email', { icon: icon('EM', '#2f6f8f'), tone: 'formal', cleanup_intensity: 'high', custom_instructions: 'Sign off with "Best, Sam". Keep paragraphs short.' }),
    context(4, 'Development', { icon: icon('DV', '#5b554a'), tone: 'casual', cleanup_intensity: 'none', custom_instructions: 'Keep code identifiers, file paths, and CLI flags exactly as spoken.' }),
  ],
  'verenu:dev-context-targets': [
    { id: 1, context_id: 2, executable: 'discord.exe', app_name: 'Discord', developer: 'Discord Inc.', platform: 'windows', created_at: ago(9000) },
    { id: 2, context_id: 4, executable: 'code.exe', app_name: 'Visual Studio Code', developer: 'Microsoft Corporation', platform: 'windows', created_at: ago(9000) },
    { id: 3, context_id: 4, executable: 'wt.exe', app_name: 'Windows Terminal', developer: 'Microsoft Corporation', platform: 'windows', created_at: ago(9000) },
  ],
  'verenu:dev-context-website-targets': [
    { id: 1, context_id: 3, domain: 'mail.google.com', created_at: ago(9000) },
    { id: 2, context_id: 2, domain: 'app.slack.com', created_at: ago(9000) },
  ],
  'verenu:dev-dictionary': [
    { id: 1, term: 'Verenu', mistake: 'Verino', auto_learned: false, correction_count: 4, confidence_tier: 'manual', last_seen_at: ago(30), created_at: ago(9000) },
    { id: 2, term: 'Tauri', mistake: 'tory', auto_learned: true, correction_count: 3, confidence_tier: 'high', last_seen_at: ago(90), created_at: ago(8000) },
    { id: 3, term: 'Kubernetes', mistake: null, auto_learned: false, correction_count: 0, confidence_tier: 'manual', last_seen_at: null, created_at: ago(7000) },
    { id: 4, term: 'SQLite', mistake: 'sequel light', auto_learned: true, correction_count: 2, confidence_tier: 'medium', last_seen_at: ago(200), created_at: ago(6000) },
  ],
  'verenu:dev-snippets': [
    { id: 1, trigger: 'my calendar link', expansion: 'https://cal.example.com/sam/30min', instructions: '', use_count: 12, created_at: ago(9000) },
    { id: 2, trigger: 'standup template', expansion: 'Yesterday:\nToday:\nBlockers:', instructions: '', use_count: 7, created_at: ago(8000) },
  ],
  'verenu:dev-context-assignments': {
    dictionary: { 1: [1], 2: [1], 3: [1], 4: [1, 2, 3, 4] },
    snippets: { 1: [1], 2: [1, 2], 3: [1], 4: [] },
  },
};

const history = [
  { id: 6, clean_text: 'The meeting moved to Thursday at 3 PM. Can you send the deck before then?', words: 15, created_at: ago(2), app_name: 'discord.exe', duration_ms: 5200 },
  { id: 5, clean_text: 'Hi Priya, thanks for the quick turnaround. I have attached the revised proposal; let me know if the timeline works for your team.', words: 23, created_at: ago(26), app_name: 'chrome.exe', duration_ms: 9100 },
  { id: 4, clean_text: 'Refactor the sync worker so retries back off exponentially, and add a test for the timeout path.', words: 17, created_at: ago(48), app_name: 'code.exe', duration_ms: 6400 },
  { id: 3, clean_text: 'Lunch at 12:30? I can grab a table at the place on Fifth.', words: 13, created_at: ago(95), app_name: 'discord.exe', duration_ms: 3900 },
  { id: 2, clean_text: 'Run cargo test with the sync filter, then rebuild the Tauri app.', words: 12, created_at: ago(140), app_name: 'wt.exe', duration_ms: 4300 },
  { id: 1, clean_text: 'Draft release notes: faster startup, smaller bundle, and local Parakeet transcription for offline dictation.', words: 15, created_at: ago(60 * 26), app_name: 'code.exe', duration_ms: 7600 },
];

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || '/usr/bin/google-chrome-stable' });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: 'light' });
page.on('pageerror', (e) => console.error('pageerror', e.message));

await page.addInitScript(({ seed, history }) => {
  if (!sessionStorage.getItem('verenu-demo-seeded')) {
    localStorage.clear();
    for (const [key, value] of Object.entries(seed)) localStorage.setItem(key, JSON.stringify(value));
    sessionStorage.setItem('verenu-demo-seeded', '1');
  }
  // History and stats are not stored by the preview backend, so answer them here.
  globalThis.__verenuDemoInvoke = (command) => {
    if (command === 'get_recent') return history;
    if (command === 'get_stats') return { total_words: 279134, avg_wpm: 148, day_streak: 5 };
    if (command === 'get_memory_mb') return 74;
    return undefined;
  };
}, { seed, history });

// Inject a tiny hook at the top of the preview backend's dispatcher.
await page.route(/\/src\/lib\/tauri\.dev\.ts(\?.*)?$/, async (route) => {
  const response = await route.fetch();
  const body = await response.text();
  const patched = body.replace(
    /(export\s+async\s+function\s+devInvoke\s*\([^)]*\)\s*\{)/,
    '$1\n  { const demo = globalThis.__verenuDemoInvoke?.(command, args); if (demo !== undefined) return demo; }',
  );
  if (patched === body) throw new Error('Could not hook devInvoke in tauri.dev.ts');
  await route.fulfill({ response, body: patched });
});

const settle = (ms = 900) => page.waitForTimeout(ms);
const shot = async (name) => {
  await settle();
  await page.screenshot({ path: join(outDir, `${name}.png`) });
  console.log('captured', name);
};
const nav = (name) => page.getByRole('button', { name, exact: true }).first().click();

await page.clock.install({ time: now });
await page.goto(baseUrl);
await settle(2500);
await shot('home');

await nav('Insights');
await shot('insights');

await nav('Team chat');
await shot('context-team-chat');

await nav('Development');
await shot('context-development');

await nav('Email');
await shot('context-email');

await nav('Style');
await shot('style');

await nav('Settings');
await nav('Models');
await shot('settings-models');

await nav('Providers');
await shot('settings-providers');

await nav('Privacy');
await shot('settings-privacy');

await browser.close();
