const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

export function nativePageIdentity(log, { kind, port, active }) {
  const identities = new Map();
  for (const line of log.split('\n')) {
    const match = /^VERENU_NATIVE_PAGE pid=(\d+) start=(\d+) page=(\d+) kind=(main|pill|other) port=(\d+)$/.exec(line);
    const gone = /^VERENU_NATIVE_PAGE_GONE pid=(\d+) page=(\d+)$/.exec(line);
    if (match) {
      const [, pid, start, page, reportedKind, reportedPort] = match;
      identities.set(`${pid}:${start}:${page}`, { pid: Number(pid), start, page, kind: reportedKind, port: Number(reportedPort) });
    } else if (gone) {
      for (const [key, item] of identities) if (item.pid === Number(gone[1]) && item.page === gone[2]) identities.delete(key);
    }
  }
  const matches = [...identities.values()].filter(item => item.kind === kind && item.port === port && active.some(process => process.pid === item.pid && process.start === item.start));
  if (matches.length > 1) throw new Error(`Ambiguous native ${kind} page identity`);
  return matches[0] || null;
}

export function nativeNavigationState(log, { kind, port, active, after = 0, expectedPage }) {
  const identity = nativePageIdentity(log, { kind, port, active });
  if (!identity) return false;
  if (expectedPage && (expectedPage.page !== identity.page || expectedPage.pid !== identity.pid || expectedPage.start !== identity.start)) throw new Error(`Native ${kind} page identity changed during navigation`);
  const events = [...log.slice(after).matchAll(/^VERENU_NAV page=(\d+) event=(\d+|failed)$/gm)].filter(match => match[1] === identity.page).map(match => match[2]);
  let started = false, finished = false;
  for (const event of events) {
    if (event === '0') {
      if (started && !finished) throw new Error(`Overlapping native ${kind} navigations`);
      started = true; finished = false;
    } else if (started && event === 'failed') throw new Error(`Native ${kind} navigation failed`);
    else if (started && event === '3') finished = true;
  }
  return finished ? identity : false;
}

export async function waitForNativeNavigation(read, options, { deadlineMs = 15000, sleep = pause } = {}) {
  const deadline = Date.now() + deadlineMs;
  while (Date.now() < deadline) {
    const { log, active } = await read();
    const page = nativeNavigationState(log, { ...options, active });
    if (page) return page;
    await sleep(100);
  }
  throw new Error(`Native ${options.kind} navigation did not finish`);
}
