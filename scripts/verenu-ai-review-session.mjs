import { createReadStream } from "node:fs";
import { readdir } from "node:fs/promises";
import path from "node:path";
import { createInterface } from "node:readline";
import { fallbackReason } from "./verenu-ai-review-logic.mjs";

// OCR 1.7.1 hides subtask API errors in JSON mode. Its private session JSONL
// retains them even when stderr only says "all file reviews failed".
// Public helpers return categories or allowlisted review findings only.
// Never expose prompts, free response text, or raw API errors.
async function* readSessionRecords(ocrHome) {
  const sessions = path.join(ocrHome, ".opencodereview", "sessions");
  try {
    for (const repo of await readdir(sessions, { withFileTypes: true })) {
      if (!repo.isDirectory()) continue;
      const directory = path.join(sessions, repo.name);
      for (const file of await readdir(directory, { withFileTypes: true })) {
        if (!file.isFile() || !file.name.endsWith(".jsonl")) continue;
        const stream = createReadStream(path.join(directory, file.name));
        const lines = createInterface({ input: stream, crlfDelay: Infinity });
        try {
          for await (const line of lines) {
            let record;
            try { record = JSON.parse(line); } catch { continue; }
            yield record;
          }
        } finally {
          lines.close();
          stream.destroy();
        }
      }
    }
  } catch {
    // Missing/unreadable diagnostic records must not turn a failure into success.
  }
}

export async function readProviderFailureReason(ocrHome) {
  for await (const record of readSessionRecords(ocrHome)) {
    if (record?.type !== "llm_error" || typeof record.error !== "string") continue;
    const reason = fallbackReason({ code: 1, stderr: record.error });
    if (reason) return reason;
  }
  return null;
}

function parseValue(value) {
  if (typeof value !== "string") return value;
  try { return JSON.parse(value); } catch { return null; }
}

// OCR 1.7.1 persists code_comment arguments before aggregate stdout exists.
// Recover only main-task comment calls, never free response text or prompts.
// These comments may not have finished OCR's location/filter stages, so return
// them without an inferred line and keep the attempt explicitly incomplete.
export async function readPersistedFindings(ocrHome) {
  const findings = new Map();
  for await (const record of readSessionRecords(ocrHome)) {
    if (record?.type !== "llm_response" || record.taskType !== "main_task" || !Array.isArray(record.tool_calls)) continue;
    for (const call of record.tool_calls) {
      if (call?.name !== "code_comment") continue;
      const args = parseValue(call.arguments);
      const comments = parseValue(args?.comments);
      const file = record.filePath || args?.path;
      if (typeof file !== "string" || !file || !Array.isArray(comments)) continue;
      for (const comment of comments) {
        if (typeof comment?.content !== "string" || !comment.content.trim()) continue;
        const finding = { file, line: null, severity: "info", message: comment.content };
        findings.set(JSON.stringify([file, finding.message]), finding);
      }
    }
  }
  return [...findings.values()];
}
