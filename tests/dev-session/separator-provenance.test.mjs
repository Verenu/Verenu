import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('Basic preserves literal separators and distinguishes spoken punctuation', { timeout: 180_000 }, async () => {
  await runBasicCases([
    [false, 'Keep this; um, continue', 'Keep this; continue'],
    [true, 'Keep this; um, continue', 'Keep this; continue'],
    [false, 'Keep this: um; continue', 'Keep this: continue'],
    [true, 'Keep this: um; continue', 'Keep this: continue'],
    [false, 'Keep this, um; continue', 'Keep this, continue'],
    [true, 'Keep this, um; continue', 'Keep this, continue'],
    [true, '"Keep this"; um, continue', '"Keep this"; continue'],
    [true, 'Keep this; um comma continue', 'Keep this, continue'],
    [true, 'Keep this comma you know comma continue', 'Keep this, continue'],
    [true, 'hi, um semicolon um, continue', 'hi; continue'],
    [true, 'abcdefgh comma scratch that foo; um, continue', 'Foo; continue'],
    [true, 'We live in the U.S. full stop change this scratch that tomorrow', 'We live in the U.S. Tomorrow'],
    [true, 'call Dr full stop um tomorrow', 'call Dr. Tomorrow'],
  ]);
});
