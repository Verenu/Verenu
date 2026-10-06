import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { DEFAULT_MODEL, DEFAULT_FALLBACK_MODEL, shouldFallback } from "./verenu-ai-review-logic.mjs";
import { readProviderFailureReason, readPersistedFindings } from "./verenu-ai-review-session.mjs";

async function session(t, records) {
  const home = await mkdtemp(path.join(tmpdir(), "verenu-review-session-test-"));
  t.after(() => rm(home, { recursive: true, force: true }));
  const directory = path.join(home, ".opencodereview", "sessions", "fixture-repo");
  await mkdir(directory, { recursive: true });
  await writeFile(path.join(directory, "fixture.jsonl"), records.map((record) => typeof record === "string" ? record : JSON.stringify(record)).join("\n"));
  return home;
}

test("hidden OCR quota errors trigger Sonnet fallback", async (t) => {
  const home = await session(t, [
    { type: "llm_request", content: "private prompt" },
    { type: "llm_error", error: '429 {"error":{"code":"RESOURCE_EXHAUSTED","message":"private upstream details"}}' },
  ]);
  const result = { code: 1, stderr: "review failed: all 1 file review(s) failed — check your LLM configuration and API key" };
  assert.equal(shouldFallback(result, DEFAULT_MODEL, DEFAULT_FALLBACK_MODEL), false);
  result.providerFailureReason = await readProviderFailureReason(home);
  assert.equal(result.providerFailureReason, "quota");
  assert.equal(shouldFallback(result, DEFAULT_MODEL, DEFAULT_FALLBACK_MODEL), true);
});

test("unavailable provider credentials are recovered from OCR diagnostics", async (t) => {
  const home = await session(t, [{ type: "llm_error", error: "503 auth_unavailable: no auth available" }]);
  assert.equal(await readProviderFailureReason(home), "model_unavailable");
});

test("prompt and response text cannot trigger fallback", async (t) => {
  const home = await session(t, [
    { type: "llm_request", error: "quota exhausted" },
    { type: "llm_response", error: "429 model_cooldown" },
    "malformed JSON quota exhausted",
    { type: "llm_error", error: "401 invalid API key" },
    { type: "llm_error", error: "context deadline exceeded" },
  ]);
  assert.equal(await readProviderFailureReason(home), null);
});

test("missing records and another attempt's quota do not misclassify failures", async (t) => {
  const home = await session(t, [{ type: "llm_error", error: "quota exceeded" }]);
  assert.equal(await readProviderFailureReason(path.join(home, "fresh-attempt")), null);
});

test("recovers only structured main-task comments without exposing diagnostic content", async (t) => {
  const args = JSON.stringify({ path: "model-path.mjs", comments: JSON.stringify([{ content: "Fix this bug", thinking: "private reasoning" }]) });
  const record = { type: "llm_response", taskType: "main_task", filePath: "actual-file.mjs", content: "private response", tool_calls: [{ name: "code_comment", arguments: args }] };
  const home = await session(t, [
    { ...record, type: "llm_request" },
    { ...record, taskType: "plan_task" },
    { ...record, tool_calls: [{ name: "read_file", arguments: args }] },
    { ...record, tool_calls: [{ name: "code_comment", arguments: "malformed" }] },
    record, record,
    { type: "llm_error", error: "quota exhausted private details" },
  ]);
  assert.deepEqual(await readPersistedFindings(home), [{ file: "actual-file.mjs", line: null, severity: "info", message: "Fix this bug" }]);
  assert.deepEqual(await readPersistedFindings(path.join(home, "other-attempt")), []);
});
