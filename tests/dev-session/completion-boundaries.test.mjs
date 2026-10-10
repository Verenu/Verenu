import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production separator replacement, protected URLs and standalone line breaks', { timeout: 180_000 }, async () => {
  await runBasicCases([
    [true, 'Keep this, um semicolon continue', 'Keep this; continue'],
    [true, 'Keep this; um comma continue', 'Keep this, continue'],
    [true, 'Keep this, um put a colon continue', 'Keep this: continue'],
    [true, '"Keep this", um semicolon continue', '"Keep this"; continue'],
    [true, 'yes. um, tomorrow', 'yes. Tomorrow'],
    [true, '"Please", um, continue', '"Please", continue'],
    [true, 'visit example.com?query=value', 'visit example.com?query=value'],
    [true, 'visit localhost:3000?debug', 'visit localhost:3000?debug'],
    [true, 'visit example.com? tomorrow', 'visit example.com? Tomorrow'],
    [true, 'say "done"?tomorrow', 'say "done"?Tomorrow'],
    [true, 'new line', '\n'],
    [true, 'new paragraph', '\n\n'],
    [true, 'new line new paragraph', '\n\n\n'],
    [true, 'um new line', '\n'],
    [true, 'scratch that', ''],
    [true, 'um', ''],
    [false, 'new line', 'new line'],
    [false, 'new paragraph', 'new paragraph'],
    [true, '"new line"', '"new line"'],
  ], ['example.com', 'localhost:3000']);
});
