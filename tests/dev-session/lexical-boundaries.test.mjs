import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production filler and command edits preserve identifier boundaries', { timeout: 180_000 }, async () => {
  // Independent default-30 worker; existing Basic36 and boundary26 stay intact.
  await runBasicCases([
    [true, 'contact um@example.com', 'contact um@example.com'],
    [true, 'visit um.edu', 'visit um.edu'],
    [false, 'contact um@example.com', 'contact um@example.com'],
    [false, 'visit um.edu', 'visit um.edu'],
    [true, 'open /um/report.md', 'open /um/report.md'],
    [true, 'open um.txt', 'open um.txt'],
    [true, 'value um_value', 'value um_value'],
    [true, 'use um-value', 'use um-value'],
    [true, 'address a@um.edu', 'address a@um.edu'],
    [true, 'version 1.um', 'version 1.um'],
    [true, 'visit comma.com', 'visit comma.com'],
    [true, 'open semicolon.txt', 'open semicolon.txt'],
    [true, 'mail comma@example.com', 'mail comma@example.com'],
    [true, 'open /comma/file', 'open /comma/file'],
    [true, 'open /tmp/comma', 'open /tmp/comma'],
    [true, 'value comma_value', 'value comma_value'],
    [true, 'use comma-value', 'use comma-value'],
    [true, 'version 1.comma', 'version 1.comma'],
    [true, 'open new line.txt', 'open new line.txt'],
    [true, 'open scratch that.txt', 'open scratch that.txt'],
    [true, 'put a comma.com', 'put a comma.com'],
    [true, 'um, send it', 'Send it'],
    [true, 'hello comma, world', 'hello, world'],
    [true, 'hello full stop.tomorrow', 'hello. Tomorrow'],
    [true, 'hello question mark?tomorrow', 'hello? Tomorrow'],
    [true, '"um comma.com"', '"um comma.com"'],
    [true, '`um comma.com`', '`um comma.com`'],
    [true, '[[VERENU_CLIPBOARD_um comma.com]]', '[[VERENU_CLIPBOARD_um comma.com]]'],
    [true, 'greeting', 'um scratch that new line'],
    [true, 'um please new line', 'Please New Line'],
  ]);
});
