// Standalone connection to the embedded driver in our already-owned worker.
// The WDIO service supplies Tauri APIs; it never launches a second app.
export function connection(port) {
  return { hostname: '127.0.0.1', port, path: '/', logLevel: 'error', connectionRetryCount: 0, connectionRetryTimeout: 60_000, capabilities: { browserName: 'tauri', 'wdio:enforceWebDriverClassic': true } };
}
