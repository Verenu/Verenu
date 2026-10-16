#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const DEV_PORT = 1420;
const SERVER_PROBE_ATTEMPTS = 3;
const SERVER_PROBE_TIMEOUT_MS = 3000;
const DEV_URLS = [
  `http://127.0.0.1:${DEV_PORT}/`,
  `http://localhost:${DEV_PORT}/`,
  `http://[::1]:${DEV_PORT}/`,
];

const running = await findReachableServer();

if (running?.kind === 'dev') {
  console.log(`[vite-dev-server] Reusing existing dev server at ${running.url}`);
  process.exit(0);
}

if (running?.kind === 'not-dev') {
  console.error(
    `[vite-dev-server] ${running.url} is serving Verenu, but it is not a dev server ` +
      '(no HMR client in the HTML) - most likely `vite preview`, which serves whatever ' +
      'was last built into dist/.',
  );
  console.error(
    '[vite-dev-server] Tauri would load that stale build, so every source edit would ' +
      'appear to do nothing. Stop that server, then start `npm run tauri dev` again.',
  );
  process.exit(1);
}

const viteEntry = fileURLToPath(new URL('../node_modules/vite/bin/vite.js', import.meta.url));
const child = spawn(process.execPath, [viteEntry], {
  stdio: 'inherit',
  shell: false,
  env: process.env,
  windowsHide: true,
});

for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
  process.on(signal, () => {
    child.kill(signal);
  });
}

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 0);
});

async function findReachableServer() {
  for (let attempt = 0; attempt < SERVER_PROBE_ATTEMPTS; attempt += 1) {
    for (const url of DEV_URLS) {
      const kind = await classifyServer(url);
      if (kind) {
        return { url, kind };
      }
    }
    if (attempt < SERVER_PROBE_ATTEMPTS - 1) {
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }

  return null;
}

async function classifyServer(url) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), SERVER_PROBE_TIMEOUT_MS);
  try {
    const response = await fetch(url, {
      method: 'GET',
      signal: controller.signal,
    });
    if (!response.ok) {
      return null;
    }

    const html = await response.text();
    if (!html.includes('<title>Verenu</title>')) {
      return null;
    }

    return html.includes('/@vite/client') ? 'dev' : 'not-dev';
  } catch {
    return null;
  } finally {
    clearTimeout(timeout);
  }
}
