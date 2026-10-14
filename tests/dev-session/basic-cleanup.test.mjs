import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';
import { basicCases } from './helpers/basic-cases.mjs';

test("production Basic and opt-in commands bypass cleanup HTTP and save exact completion output", { timeout: 180_000 }, async () => {
  await runBasicCases(basicCases);
});
