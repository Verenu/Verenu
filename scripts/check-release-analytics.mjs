const token = process.env.POSTHOG_PROJECT_TOKEN ?? '';
const host = process.env.POSTHOG_HOST ?? '';

function fail(message) {
  console.error(`Release analytics preflight failed: ${message}`);
  process.exitCode = 1;
}

if (!token.trim() || token !== token.trim() || /\s/.test(token)) {
  fail('POSTHOG_PROJECT_TOKEN must contain a project ingestion token.');
} else {
  if (!host || host !== host.trim() || /\s/.test(host) || !host.startsWith('https://')) {
    fail('POSTHOG_HOST must be an HTTPS ingestion base URL.');
  }

  let parsedHost;
  if (process.exitCode !== 1) {
    try {
      parsedHost = new URL(host);
    } catch {
      fail('POSTHOG_HOST must be an HTTPS ingestion base URL.');
    }
  }

  if (parsedHost && (
    parsedHost.protocol !== 'https:' ||
    !parsedHost.hostname ||
    parsedHost.username ||
    parsedHost.password ||
    parsedHost.pathname !== '/' ||
    parsedHost.search ||
    parsedHost.hash
  )) {
    fail('POSTHOG_HOST must be an HTTPS ingestion base URL without credentials or a path.');
  }
}

if (process.exitCode !== 1) {
  console.log('Release analytics configuration is valid.');
}
