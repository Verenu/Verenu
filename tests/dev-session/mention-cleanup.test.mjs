import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';
import { mentionCases } from './helpers/basic-cases.mjs';

test("production mention commands preserve consumed sentence state and protected payloads", { timeout: 180_000 }, async () => {
  await runBasicCases(mentionCases);
});
