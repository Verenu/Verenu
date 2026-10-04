import { mount } from 'svelte';
import { invoke } from './lib/tauri';
import { disableBrowserContextMenu } from './lib/disable-context-menu';
import './theme.css';
import './app.css';
import './ui.css';
import './mobile.css';
import { initializeDevSession, isBrowserDevSession } from './lib/devSession';

disableBrowserContextMenu(); // The app webview lives until the process exits.

async function start() {
  if (import.meta.env.DEV && import.meta.env.VITE_VERENU_NATIVE_TEST === '1') await import('@wdio/tauri-plugin');
  // The worker WebView exists only to supply Tauri command context. It must
  // not run a second UI's polling, notifications, or automatic update checks.
  if (import.meta.env.VITE_VERENU_SESSION === '1' && !isBrowserDevSession()) return;
  let App: typeof import('./App.svelte').default;
  try {
    await initializeDevSession();
    App = (await import('./App.svelte')).default;
  } catch (error) {
    const root = document.getElementById('app') as HTMLElement;
    const page = document.createElement('main');
    page.style.cssText = 'max-width:34rem;margin:15vh auto;padding:1.5rem;display:grid;gap:1rem';
    const devSession = isBrowserDevSession();
    const heading = document.createElement('h1'); heading.textContent = devSession ? 'Connect to your dev session' : 'Unable to load Verenu';
    const message = document.createElement('p'); message.textContent = devSession
      ? (error instanceof Error ? error.message : 'Dev backend is unavailable')
      : 'Reload the app to try again.';
    const retry = document.createElement('button'); retry.className = 'btn-primary'; retry.textContent = devSession ? 'Try connecting again' : 'Reload app'; retry.onclick = () => location.reload();
    page.append(heading, message);
    if (devSession) {
      const help = document.createElement('p'); help.textContent = 'Open the access link in this session’s private access.json file. If the backend is still building, try again once it reports ready.';
      page.append(help);
    }
    page.append(retry); root.replaceChildren(page);
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
