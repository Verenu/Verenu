import { createReadStream } from "node:fs";
import { readdir } from "node:fs/promises";
import path from "node:path";
import { createInterface } from "node:readline";
import { fallbackReason } from "./verenu-ai-review-logic.mjs";

// OCR 1.7.1 hides subtask API errors in JSON mode. Its private session JSONL
// retains them even when stderr only says "all file reviews failed".
// Return a category only. Never expose prompts, responses, or raw API errors.
export async function readProviderFailureReason(ocrHome) {
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
            if (record?.type !== "llm_error" || typeof record.error !== "string") continue;
            const reason = fallbackReason({ code: 1, stderr: record.error });
            if (reason) return reason;
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
  return null;
}
