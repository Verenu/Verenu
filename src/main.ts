import { mount } from 'svelte';
import App from './App.svelte';
import { invoke } from './lib/tauri';
import { disableBrowserContextMenu } from './lib/disable-context-menu';
import './theme.css';
import './app.css';
import './ui.css';
import { initializeDevSession, isBrowserDevSession } from './lib/devSession';

disableBrowserContextMenu(); // The app webview lives until the process exits.

async function start() {
  if (import.meta.env.DEV && import.meta.env.VITE_VERENU_NATIVE_TEST === '1') await import('@wdio/tauri-plugin');
  // The worker WebView exists only to supply Tauri command context. It must
  // not run a second UI's polling, notifications, or automatic update checks.
  if (import.meta.env.VITE_VERENU_SESSION === '1' && !isBrowserDevSession()) return;
  try { await initializeDevSession(); } catch (error) {
    const root = document.getElementById('app') as HTMLElement;
    const page = document.createElement('main');
    page.style.cssText = 'max-width:34rem;margin:15vh auto;padding:1.5rem;display:grid;gap:1rem';
    const heading = document.createElement('h1'); heading.textContent = 'Connect to your dev session';
    const message = document.createElement('p'); message.textContent = error instanceof Error ? error.message : 'Dev backend is unavailable';
    const help = document.createElement('p'); help.textContent = 'Open the access link in this session’s private access.json file. If the backend is still building, try again once it reports ready.';
    const retry = document.createElement('button'); retry.className = 'btn-primary'; retry.textContent = 'Try connecting again'; retry.onclick = () => location.reload();
    page.append(heading, message, help, retry); root.replaceChildren(page);
    return;
  }
  const app = mount(App, { target: document.getElementById('app') as HTMLElement });
  void invoke('frontend_ready').catch(() => {
    if (isBrowserDevSession()) window.dispatchEvent(new CustomEvent('verenu:dev-connection', { detail: 'The Rust backend startup handshake failed.' }));
    else console.error('Failed to complete startup handshake.');
  });
  return app;
}
const app = start();

export default app;
