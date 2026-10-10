import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production quoted contractions keep fillers and voice commands literal', { timeout: 180_000 }, async () => {
  // Independent default-30 worker; all existing scenario groups stay intact.
  await runBasicCases([
    [true, "say 'don't um pause' literally", "say 'don't um pause' literally"],
    [false, "say 'don't um pause' literally", "say 'don't um pause' literally"],
    [true, 'say ‘don’t um pause’ literally', 'say ‘don’t um pause’ literally'],
    [true, "say 'don't new line please' literally", "say 'don't new line please' literally"],
    [false, "say 'don't new line please' literally", "say 'don't new line please' literally"],
    [true, 'say ‘don’t new line please’ literally', 'say ‘don’t new line please’ literally'],
    [true, "say 'don't scratch that please' literally", "say 'don't scratch that please' literally"],
    [true, 'say ‘l’été new line please’ literally', 'say ‘l’été new line please’ literally'],
    [true, "say 'cafe\u0301's um pause' literally", "say 'cafe\u0301's um pause' literally"],
    [true, "say 'don't um pause", "say 'don't um pause"],
    [true, 'say ‘don’t um pause', 'say ‘don’t um pause'],
    [true, "say 'don't new line please", "say 'don't new line please"],
    [true, 'say ‘don’t new line please', 'say ‘don’t new line please'],
    [true, "say 'don't new line' comma tomorrow", "say 'don't new line', tomorrow"],
    [true, 'say ‘don’t new line’ comma tomorrow', 'say ‘don’t new line’, tomorrow'],
    [true, 'say "don\'t um new line" literally', 'say "don\'t um new line" literally'],
    [true, "say `don't um new line` literally", "say `don't um new line` literally"],
    [true, "[[VERENU_CLIPBOARD_don't um new line]]", "[[VERENU_CLIPBOARD_don't um new line]]"],
    [true, 'greeting', 'um scratch that new line'],
    [true, 'um please new line', 'Please New Line'],
  ]);
});
