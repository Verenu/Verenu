import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production commands preserve URL queries and roll back whole identifiers', { timeout: 180_000 }, async () => {
  await runBasicCases([
    [true, 'visit example.com?query=value', 'visit example.com?query=value'],
    [true, 'visit https://example.com/path?query=value', 'visit https://example.com/path?query=value'],
    [true, 'open report.md!section', 'open report.md!section'],
    [true, 'visit example.com?query', 'visit example.com?query'],
    [true, 'visit example.com?query=value scratch that', ''],
    [true, 'visit https://example.com/path?query=value scratch that', ''],
    [true, 'open report.md!section scratch that', ''],
    [true, 'visit example.com?query scratch that', ''],
    [true, 'Hello. visit example.com?query=value scratch that tomorrow', 'Hello. Tomorrow'],
    [true, 'Hello. open report.md!section scratch that tomorrow', 'Hello. Tomorrow'],
    [true, 'hello!tomorrow', 'hello!Tomorrow'],
    [true, 'Okay? tomorrow', 'Okay? Tomorrow'],
    [false, 'visit example.com?query=value scratch that', 'visit example.com?query=value scratch that'],
    [true, '"visit example.com?query=value scratch that"', '"visit example.com?query=value scratch that"'],
  ]);
});
