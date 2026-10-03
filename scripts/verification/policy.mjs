export const categories = ['static', 'unit', 'rust', 'renderer', 'session', 'pipeline', 'native', 'native-integration', 'migration', 'inspection'];
export function requirements(files, extra = []) {
  const required = new Set();
  if (files.some((file) => /^(src\/|scripts\/|tests\/|package|vite|playwright|src-tauri\/)/.test(file))) {
    required.add('static'); required.add('unit');
  }
  if (files.some((file) => /^(src\/|vite|package)/.test(file))) {
    required.add('renderer'); required.add('session');
  }
  if (files.some((file) => /^src\/.*\.(svelte|css)$/.test(file))) required.add('inspection');
  if (files.some((file) => /^src-tauri\//.test(file))) required.add('rust');
  if (files.some((file) => /^src-tauri\/src\/(commands|data|dev_session)/.test(file))) required.add('session');
  if (files.some((file) => /^src-tauri\/src\/(pipeline|api\/(cleanup|transcription|prompts)|media|local_stt|local_llm)/.test(file))) required.add('pipeline');
  if (files.some((file) => /^src-tauri\/src\/(core|system|permissions)|^src-tauri\/(capabilities|Entitlements|Info)|^src\/.*(Setup|permission)/i.test(file))) {
    required.add('native'); required.add('native-integration');
  }
  if (files.some((file) => /^src-tauri\/src\/data\/db/.test(file))) required.add('migration');
  for (const category of extra) {
    if (!categories.includes(category)) throw new Error(`Unknown verification category: ${category}`);
    required.add(category);
  }
  return [...required];
}
export function evaluate(required, records, identity, acceptance = []) {
  const issues = [];
  for (const category of required) {
    const matching = records.filter((record) => record.category === category);
    if (!matching.length) { issues.push({ category, status: 'incomplete', reason: 'No evidence recorded' }); continue; }
    for (const record of matching) {
      if (record.status === 'failed') issues.push({ category, status: 'failed', reason: record.reason || 'Check failed' });
      else if (record.status !== 'passed') issues.push({ category, status: 'incomplete', reason: record.reason || 'Check did not pass' });
      if (record.fingerprint !== identity.fingerprint || record.worktree !== identity.worktree) issues.push({ category, status: 'incomplete', reason: 'Evidence is stale or belongs to another worktree' });
      if (record.flaky) issues.push({ category, status: 'incomplete', reason: 'Check needed retries' });
    }
  }
  for (const criterion of acceptance) {
    const outcomes = records.filter((record) => record.category === 'acceptance' && record.criterion === criterion.id);
    if (outcomes.some((record) => record.status === 'failed')) issues.push({ category: 'acceptance', status: 'failed', reason: `Acceptance criterion failed: ${criterion.id}` });
    if (!criterion.id || !criterion.expected || !outcomes.some((record) => record.status === 'passed' && record.fingerprint === identity.fingerprint && record.worktree === identity.worktree && record.observed && record.artifacts?.length)) {
      issues.push({ category: 'acceptance', status: 'incomplete', reason: `Acceptance criterion lacks current outcome evidence: ${criterion.id || '(missing id)'}` });
    }
  }
  return { status: issues.some((issue) => issue.status === 'failed') ? 'failed' : issues.length ? 'incomplete' : 'verified', issues };
}
