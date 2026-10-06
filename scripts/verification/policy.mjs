export const categories = ['static', 'unit', 'rust', 'renderer', 'session', 'pipeline', 'native', 'native-integration', 'migration', 'inspection', 'ci', 'android'];
export const nativeScopes = ['shortcuts', 'insertion', 'clipboard', 'focus', 'permissions', 'microphone', 'focused-text', 'window-identity', 'media-control'];
export function incompleteUnlessFailed(status) { return status === 'failed' ? 'failed' : 'incomplete'; }
export function requirements(files, extra = []) {
  const required = new Set();
  if (files.some((file) => /^(src\/|scripts\/|tests\/|package|vite|playwright|src-tauri\/|\.github\/|\.cargo\/|.*\.html$)/.test(file))) {
    required.add('static'); required.add('unit');
  }
  if (files.some((file) => /^(src\/|vite|package|tests\/(browser|dev-session)\/|scripts\/test-owned-session|.*\.html$)/.test(file))) {
    required.add('renderer'); required.add('session');
  }
  if (files.some((file) => /^src\/.*\.(svelte|css)$|^[^/]+\.html$/.test(file))) required.add('inspection');
  if (files.some((file) => /^src-tauri\/(?!android\/)/.test(file) || /^\.cargo\//.test(file))) required.add('rust');
  if (files.some((file) => /^src-tauri\/src\/(commands|data|dev_session)/.test(file))) required.add('session');
  if (files.some((file) => /^src-tauri\/src\/(pipeline|api\/(cleanup|transcription|prompts)|media|local_stt|local_llm)/.test(file))) required.add('pipeline');
  if (files.some((file) => /^src-tauri\/src\/(core|system|permissions)|^src-tauri\/(capabilities|Entitlements|Info)|^src\/.*(Setup|permission)/i.test(file))) {
    required.add('native'); required.add('native-integration');
  }
  if (files.some((file) => /^src-tauri\/src\/data\/db/.test(file))) required.add('migration');
  if (files.some((file) => /^src-tauri\/src\/pipeline\/pill|^pill\.html$|^src\/.*[Pp]ill/.test(file))) required.add('native');
  if (files.some((file) => /^\.github\/|^scripts\/ci\//.test(file))) required.add('ci');
  if (files.some((file) => /^src-tauri\/(android\/|src\/android\/|tauri\.android)|^scripts\/android|^\.cargo\//.test(file))) required.add('android');
  for (const category of extra) {
    if (!categories.includes(category)) throw new Error(`Unknown verification category: ${category}`);
    required.add(category);
  }
  return [...required];
}
export function nativeRequirements(files, extra = []) {
  const result = [...extra];
  for (const file of files) {
    let scopes = [];
    if (/hotkey|shortcut/.test(file)) scopes = ['shortcuts'];
    else if (/injection/.test(file)) scopes = ['insertion', 'clipboard', 'focus'];
    else if (/permission|Entitlements|Info\.plist|Setup/i.test(file)) scopes = ['permissions'];
    else if (/media\/audio/.test(file)) scopes = ['microphone'];
    else if (/core\/(hyprland|window|foreground)/.test(file)) scopes = ['window-identity'];
    else if (/system\/media_control/.test(file)) scopes = ['media-control'];
    else if (/auto_learn\/focused_text/.test(file)) scopes = ['focused-text'];
    if (!scopes.length) continue;
    const platforms = /android/.test(file) ? ['android'] : /linux|hyprland/.test(file) ? ['linux']
      : /macos|\/mac\.|Entitlements|Info\.plist/.test(file) ? ['darwin']
        : /windows|\/win\./.test(file) ? ['win32'] : ['linux', 'darwin', 'win32'];
    for (const platform of platforms) for (const scope of scopes) result.push({ platform, scope });
  }
  for (const row of result) {
    if (!['linux', 'darwin', 'win32', 'android'].includes(row.platform) || !nativeScopes.includes(row.scope)) throw new Error('Invalid native platform or capability requirement');
  }
  return result.filter((row, index) => result.findIndex(other => other.platform === row.platform && other.scope === row.scope) === index);
}
export function evaluate(required, records, identity, acceptance = [], native = []) {
  const issues = [];
  for (const category of required) {
    const matching = records.filter((record) => record.category === category);
    if (!matching.length) { issues.push({ category, status: 'incomplete', reason: 'No evidence recorded' }); continue; }
    for (const record of matching) {
      if (record.status === 'failed') issues.push({ category, status: 'failed', reason: record.reason || 'Check failed' });
      else if (record.status !== 'passed') issues.push({ category, status: 'incomplete', reason: record.reason || 'Check did not pass' });
      if (record.fingerprint !== identity.fingerprint || record.worktree !== identity.worktree) issues.push({ category, status: 'incomplete', reason: 'Evidence is stale or belongs to another worktree' });
      if (record.flaky) issues.push({ category, status: 'incomplete', reason: 'Check needed retries' });
      if (category === 'native-integration' && (!['linux', 'darwin', 'win32', 'android'].includes(record.platform) || !record.scope?.some(scope => nativeScopes.includes(scope)))) {
        issues.push({ category, status: 'incomplete', reason: 'Native evidence needs a platform and an OS capability' });
      }
    }
  }
  for (const { platform, scope } of native) {
    if (!records.some(record => record.category === 'native-integration' && record.status === 'passed' && !record.flaky && record.platform === platform && record.scope?.includes(scope) && record.fingerprint === identity.fingerprint && record.worktree === identity.worktree)) {
      issues.push({ category: 'native-integration', status: 'incomplete', reason: `No current ${platform} evidence for ${scope}` });
    }
  }
  for (const criterion of acceptance) {
    const outcomes = records.filter((record) => record.category === 'acceptance' && record.criterion === criterion.id);
    if (outcomes.some((record) => record.status === 'failed')) issues.push({ category: 'acceptance', status: 'failed', reason: `Acceptance criterion failed: ${criterion.id}` });
    if (!criterion.id || !criterion.expected || outcomes.some(record => record.flaky || record.status !== 'passed' || record.fingerprint !== identity.fingerprint || record.worktree !== identity.worktree) || !outcomes.some((record) => record.status === 'passed' && record.fingerprint === identity.fingerprint && record.worktree === identity.worktree && record.observed && record.artifacts?.length)) {
      issues.push({ category: 'acceptance', status: 'incomplete', reason: `Acceptance criterion lacks current outcome evidence: ${criterion.id || '(missing id)'}` });
    }
  }
  return { status: issues.some((issue) => issue.status === 'failed') ? 'failed' : issues.length ? 'incomplete' : 'verified', issues };
}
