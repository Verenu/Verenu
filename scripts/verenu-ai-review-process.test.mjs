import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { performance } from "node:perf_hooks";
import { runReviewProcess } from "./verenu-ai-review-process.mjs";
import { failureExitCode } from "./verenu-ai-review-logic.mjs";

test("stops hung OCR on a hidden structured quota error before internal retries finish", async (t) => {
  const home = await mkdtemp(path.join(tmpdir(), "verenu-review-process-test-"));
  t.after(() => rm(home, { recursive: true, force: true }));
  const script = `
    const fs = require('node:fs');
    const path = require('node:path');
    const dir = path.join(process.argv[1], '.opencodereview', 'sessions', 'repo');
    fs.mkdirSync(dir, {recursive:true});
    fs.writeFileSync(path.join(dir, 'session.jsonl'), JSON.stringify({type:'llm_error', error:'RESOURCE_EXHAUSTED: private detail'}) + '\\n');
    process.on('SIGTERM', () => {});
    setInterval(() => {}, 1000);
  `;
  const started = performance.now();
  const result = await runReviewProcess(process.execPath, ["-e", script, home], {
    ocrHome: home, pollMs: 20, timeoutMs: 10_000, killGraceMs: 50,
  });
  assert.equal(result.providerFailureReason, "quota");
  assert.equal(result.code, 1);
  assert.equal(result.timedOut, false);
  assert.equal(failureExitCode(result), 0);
  assert.ok(performance.now() - started < 5000);
  assert.doesNotMatch(JSON.stringify(result), /private detail/);
});

test("deadline stops a silent hung provider and remains nonblocking", async () => {
  const result = await runReviewProcess(process.execPath, ["-e", "setInterval(() => {}, 1000)"], {
    timeoutMs: 200, killGraceMs: 50,
  });
  assert.equal(result.timedOut, true);
  assert.equal(result.code, 1);
  assert.equal(failureExitCode(result), 0);
});

test("normal output and unrelated failures keep their original result", async () => {
  const good = await runReviewProcess(process.execPath, ["-e", 'console.log("quota exhausted in reviewed source")'], { timeoutMs: 5000 });
  assert.equal(good.code, 0);
  assert.equal(good.timedOut, false);
  assert.equal(good.providerFailureReason, null);
  const bad = await runReviewProcess(process.execPath, ["-e", 'console.error("invalid config"); process.exitCode = 2'], { timeoutMs: 5000 });
  assert.equal(bad.code, 2);
  assert.equal(failureExitCode(bad), 1);
});

test("spawn failures reject and clean up timers", async () => {
  await assert.rejects(runReviewProcess("/nonexistent/verenu-review-test", [], { timeoutMs: 5000 }), { code: "ENOENT" });
});
