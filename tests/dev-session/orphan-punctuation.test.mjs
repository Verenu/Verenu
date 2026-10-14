import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production filler removal discards leading orphan marks and suppresses empty output', { timeout: 180_000 }, async () => {
  await runBasicCases([
    [false, '(um), continue', 'Continue'],
    [false, '[um]; continue', 'Continue'],
    [false, '{um}: continue', 'Continue'],
    [false, '([um]), continue', 'Continue'],
    [false, '(um).', ''],
    [false, '(um)!', ''],
    [false, '(um)?', ''],
    [false, '(um). continue', 'Continue'],
    [false, '(um)\ncontinue', '\nContinue'],
    [true, '(um), continue', 'Continue'],
    [true, '(um).', ''],
    [false, 'Please (um), continue', 'Please, continue'],
    [false, 'Please (um). continue', 'Please. Continue'],
    [true, '"(um), continue"', '"(um), continue"'],
    [true, '`(um).`', '`(um).`'],
    [false, '"Please" (um), continue', '"Please", continue'],
    [false, '`Please` (um), continue', '`Please`, continue'],
  ]);
});
