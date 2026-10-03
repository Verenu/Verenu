export async function createNativeSession(port, {
  fetchImpl = fetch,
  deadlineMs = 15_000,
  delayMs = 200,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
} = {}) {
  const deadline = Date.now() + deadlineMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const response = await fetchImpl(`http://127.0.0.1:${port}/session`, {
        method: 'POST', headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ capabilities: { alwaysMatch: { 'wdio:tauriServiceOptions': { windowLabel: 'main' } } } }),
      });
      const body = await response.json();
      if (!response.ok || !body.value?.sessionId) {
        throw new Error(`Could not start native WebDriver session: ${body.value?.message || response.status}`);
      }
      return body.value.sessionId;
    } catch (error) {
      lastError = error;
      await sleep(delayMs);
    }
  }
  throw new Error(`Native WebDriver session did not become ready: ${lastError?.message || 'timed out'}`);
}
