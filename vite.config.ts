import { execSync } from 'node:child_process';
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import pkg from './package.json';

function git(command: string): string {
  try {
    return execSync(command, { stdio: ['ignore', 'pipe', 'ignore'] })
      .toString()
      .trim();
  } catch {
    return '';
  }
}

const gitSha = git('git rev-parse --short HEAD') || 'unknown';
const gitBranch = git('git rev-parse --abbrev-ref HEAD') || 'unknown';
const gitDirty = git('git status --porcelain') !== '';
const buildTime = new Date().toISOString();

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
    __VERENU_GIT_SHA__: JSON.stringify(gitSha),
    __VERENU_GIT_BRANCH__: JSON.stringify(gitBranch),
    __VERENU_GIT_DIRTY__: JSON.stringify(gitDirty),
    __VERENU_BUILD_TIME__: JSON.stringify(buildTime),
  },
  server: {
    port: 1420,
    strictPort: true,
    host: '127.0.0.1',
    headers: {
      'Cache-Control': 'no-store',
    },
    watch: {
      ignored: ['**/src-tauri/**', '**/installers/**', '**/dist/**', '**/.git/**'],
    },
  },
  build: {
    target: ['es2020', 'chrome100'],
    minify: !process.env.TAURI_DEBUG,
    sourcemap: !!process.env.TAURI_DEBUG,
    rollupOptions: {
      input: {
        main: 'index.html',
        pill: 'pill.html',
      },
    },
  },
});
