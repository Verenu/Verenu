import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { DEFAULT_MODEL, DEFAULT_FALLBACK_MODEL, shouldFallback } from "./verenu-ai-review-logic.mjs";
import { readProviderFailureReason } from "./verenu-ai-review-session.mjs";

async function session(t, records) {
  const home = await mkdtemp(path.join(tmpdir(), "verenu-review-session-test-"));
  t.after(() => rm(home, { recursive: true, force: true }));
  const directory = path.join(home, ".opencodereview", "sessions", "fixture-repo");
  await mkdir(directory, { recursive: true });
  await writeFile(path.join(directory, "fixture.jsonl"), records.map((record) => typeof record === "string" ? record : JSON.stringify(record)).join("\n"));
  return home;
}

test("hidden OCR quota errors trigger Sonnet low fallback", async (t) => {
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
