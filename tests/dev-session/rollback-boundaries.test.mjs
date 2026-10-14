import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production rollback preserves only real clause boundaries and protected payloads', { timeout: 180_000 }, async () => {
  // Independent default-30 worker; existing scenario groups stay unchanged.
  await runBasicCases([
    [true, 'open report.md scratch that', ''],
    [true, 'visit example.com scratch that', ''],
    [true, 'version 1.25 scratch that', ''],
    [true, 'visit https://example.com scratch that', ''],
    [true, 'open /tmp/report.md scratch that', ''],
    [true, 'visit https://example.com/report.md scratch that', ''],
    [true, 'Hi team. open report.md scratch that tomorrow', 'Hi team. Tomorrow'],
    [true, 'Hi team. version 1.25 scratch that tomorrow', 'Hi team. Tomorrow'],
    [true, 'Keep this: visit https://example.com scratch that tomorrow', 'Keep this: Tomorrow'],
    [true, 'Keep it; visit example.com scratch that tomorrow', 'Keep it; Tomorrow'],
    [true, 'Hello… world scratch that', 'Hello…'],
    [true, 'Hello… world scratch that tomorrow', 'Hello… Tomorrow'],
    [true, 'Hello!world scratch that tomorrow', 'Hello! Tomorrow'],
    [true, 'Hello. scratch that', ''],
    [true, 'Hello… scratch that', ''],
    [true, 'Keep this put a colon scratch that', ''],
    [false, 'open report.md scratch that', 'open report.md scratch that'],
    [false, 'visit https://example.com scratch that', 'visit https://example.com scratch that'],
    [true, '"open report.md scratch that"', '"open report.md scratch that"'],
    [true, '[[VERENU_CLIPBOARD_report.md]] scratch that', '[[VERENU_CLIPBOARD_report.md]] '],
  ]);
});
