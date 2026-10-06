import fs from 'node:fs';
import { execFileSync } from 'node:child_process';
import { requiredCiRulesetPayload } from './ruleset-payload.mjs';

// Run --apply only after the aggregate check is available on master.
const contexts = ['Frontend checks', 'Dependency audits', 'Rust checks (windows-latest)', 'Rust checks (macos-latest)', 'No large files', 'All-in-one fast profile', 'CI required', 'Review dependency changes', 'review'];
const gh = args => JSON.parse(execFileSync('gh', args, { encoding: 'utf8', timeout: 30_000 }));
const repository = gh(['repo', 'view', '--json', 'nameWithOwner']).nameWithOwner;
const endpoint = `repos/${repository}/rulesets`;
const name = 'Verenu required CI';
const list = gh(['api', endpoint]);
const existing = list.find(row => row.name === name);
if (process.argv.includes('--apply')) {
  const current = existing ? gh(['api', `${endpoint}/${existing.id}`]) : null;
  const payload = requiredCiRulesetPayload(current, contexts);
  // Keep a local rollback copy without changing the broader repository rules.
  fs.mkdirSync('test-results', { recursive: true });
  fs.writeFileSync('test-results/merge-rules-before.json', JSON.stringify(current, null, 2), { mode: 0o600 });
  execFileSync('gh', ['api', existing ? `${endpoint}/${existing.id}` : endpoint, '--method', existing ? 'PUT' : 'POST', '--input', '-'], { input: JSON.stringify(payload), stdio: ['pipe', 'ignore', 'inherit'], timeout: 30_000 });
}
const effective = gh(['api', `repos/${repository}/rules/branches/master`]);
const checks = effective.filter(row => row.type === 'required_status_checks').flatMap(row => row.parameters.required_status_checks);
const missing = contexts.filter(context => !checks.some(row => row.context === context && row.integration_id === 15368));
if (missing.length) {
  console.error(`Required CI checks missing: ${missing.join(', ')}`);
  process.exitCode = 1;
} else console.log('Master requires all existing CI checks and the AI review, bound to GitHub Actions.');
