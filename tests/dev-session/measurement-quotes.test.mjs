import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production quoted measurements retain literal speech and separate quoted spans', { timeout: 180_000 }, async () => {
  await runBasicCases([
    [true, 'say "the board is 5" new line please" literally', 'say "the board is 5" new line please" literally'],
    [true, 'say "the board is 5" um please" literally', 'say "the board is 5" um please" literally'],
    [true, 'say "the board is 5" scratch that please" literally', 'say "the board is 5" scratch that please" literally'],
    [true, 'say "the board is 5.5" new line please" literally', 'say "the board is 5.5" new line please" literally'],
    [true, 'say "the board is 5" by 6" new line please" literally', 'say "the board is 5" by 6" new line please" literally'],
    [true, 'say "size 5" new line tomorrow', 'say "size 5"\nTomorrow'],
    [true, 'say "size 5" new line "um please"', 'say "size 5"\n"um please"'],
    [true, 'say "5" new line "6"', 'say "5"\n"6"'],
    [false, 'say "size 5" new line tomorrow', 'say "size 5" new line tomorrow'],
    [true, '`the board is 5" new line please`', '`the board is 5" new line please`'],
  ]);
});
