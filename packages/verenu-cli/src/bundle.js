const allowed = new Set(['launch', 'settings', 'sqlite', 'cache', 'models', 'keyring', 'provider-keys', 'terms', 'network', 'process', 'logs']);
export function bundle(results) {
  return JSON.stringify({ version: 1, checks: results.filter(r => allowed.has(r.name) && ['pass', 'warn', 'fail'].includes(r.status)).map(({ name, status }) => ({ name, status })) }, null, 2);
}
