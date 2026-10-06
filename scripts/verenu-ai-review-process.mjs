import { spawn } from "node:child_process";
import { readProviderFailureReason } from "./verenu-ai-review-session.mjs";

export const REVIEW_TIMEOUT_MS = 3 * 60 * 1000;
export const PREVIEW_TIMEOUT_MS = 30 * 1000;

// Watch private OCR diagnostics during execution, before its internal retries
// finish. Only structured llm_error categories can stop a provider attempt.
export function runReviewProcess(cmd, args, {
  cwd, env, ocrHome, timeoutMs = REVIEW_TIMEOUT_MS, pollMs = 1000,
  killGraceMs = 1000,
} = {}) {
  return new Promise((resolve, reject) => {
    const grouped = process.platform !== "win32";
    const child = spawn(cmd, args, { cwd, env, shell: false, detached: grouped });
    let stdout = "";
    let stderr = "";
    let providerFailureReason = null;
    let timedOut = false;
    let stopped = false;
    let finished = false;
    let checking = false;
    let killTimer;
    const signal = (name) => {
      try {
        if (grouped) process.kill(-child.pid, name);
        else child.kill(name);
      } catch (err) {
        if (err.code !== "ESRCH") child.kill(name);
      }
    };
    const stop = () => {
      if (finished || stopped) return;
      stopped = true;
      signal("SIGTERM");
      killTimer = setTimeout(() => signal("SIGKILL"), killGraceMs);
    };
    const timeout = setTimeout(() => {
      timedOut = true;
      stop();
    }, timeoutMs);
    const poll = ocrHome ? setInterval(async () => {
      if (checking || stopped || finished) return;
      checking = true;
      try {
        const reason = await readProviderFailureReason(ocrHome);
        if (reason && !finished && !stopped) {
          providerFailureReason = reason;
          stop();
        }
      } finally {
        checking = false;
      }
    }, pollMs) : null;
    const cleanup = () => {
      finished = true;
      clearTimeout(timeout);
      clearInterval(poll);
      // If terminated descendants still hold pipes, close arrives only after
      // the escalation. Never signal a group after its streams have closed.
      clearTimeout(killTimer);
    };
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (data) => { stdout += data; });
    child.stderr.on("data", (data) => { stderr += data; });
    child.on("error", (err) => { cleanup(); reject(err); });
    child.on("close", (code) => {
      cleanup();
      resolve({ code: stopped ? 1 : code ?? 1, stdout, stderr, providerFailureReason, timedOut });
    });
  });
}
