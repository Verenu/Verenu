import test from 'node:test';
import { runDualSnippetCases } from './helpers/basic-cleanup.mjs';

test("dual voice commands do not admit alternate-only snippet expansions without their constraints", { timeout: 180_000 }, async () => {
  await runDualSnippetCases();
});
