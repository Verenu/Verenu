import fs from 'node:fs';
import { execFileSync } from 'node:child_process';

const contexts = ['Frontend checks', 'Dependency audits', 'Rust checks (windows-latest)', 'Rust checks (macos-latest)', 'No large files', 'All-in-one fast profile', 'Review dependency changes', 'review'];
const gh = args => JSON.parse(execFileSync('gh', args, { encoding: 'utf8', timeout: 30_000 }));
const repository = gh(['repo', 'view', '--json', 'nameWithOwner']).nameWithOwner;
const endpoint = `repos/${repository}/rulesets`;
const name = 'Verenu required CI';
const list = gh(['api', endpoint]);
const existing = list.find(row => row.name === name);
if (process.argv.includes('--apply')) {
  const current = existing ? gh(['api', `${endpoint}/${existing.id}`]) : null;
  const payload = {
    name, target: 'branch', enforcement: 'active',
    conditions: { ref_name: { include: ['refs/heads/master'], exclude: [] } },
    bypass_actors: current?.bypass_actors || [{ actor_type: 'OrganizationAdmin', actor_id: null, bypass_mode: 'always' }],
    rules: [{ type: 'required_status_checks', parameters: {
      strict_required_status_checks_policy: true,
      required_status_checks: contexts.map(context => ({ context, integration_id: 15368 })),
    } }],
  };
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
