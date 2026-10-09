import { stopOwned } from './process.mjs';
import { executedRustTests } from './rust-summary.mjs';

export function formattingResult(checked, output) {
  const cases = [...output.matchAll(/^VERENU_FORMAT_CASE_PASSED:([1-4])$/gm)].map(match => Number(match[1]));
  if (checked.status === 'failed' && cases.length === 0
      && executedRustTests(output) === 1
      && /test result: FAILED\. 0 passed; 1 failed;/.test(output)
      && output.includes('VERENU_FIXTURE_PREREQUISITE_UNAVAILABLE:')) {
    return { status: 'incomplete', cases, reason: 'Expected fixture PID/window/focused entry was unavailable at the readiness deadline' };
  }
  if (checked.status !== 'passed') return { status: 'failed', cases, reason: checked.reason };
  if (executedRustTests(output) !== 1 || cases.join(',') !== '1,2,3,4') {
    return { status: 'failed', cases, reason: 'The native Rust fixture must execute its test and all four formatting cases' };
  }
  return { status: 'verified', cases };
}

export async function stopFixture(child, timeout = 1000) {
  if (!child?.pid || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise(resolve => child.once('exit', resolve));
  stopOwned(child);
  let timer;
  await Promise.race([exited, new Promise(resolve => { timer = setTimeout(resolve, timeout); })]);
  clearTimeout(timer);
  if (child.exitCode === null && child.signalCode === null) {
    stopOwned(child, 'SIGKILL');
    await Promise.race([exited, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Owned fixture did not exit after SIGKILL')), timeout);
    })]).finally(() => clearTimeout(timer));
  }
}
