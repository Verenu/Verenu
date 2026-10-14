import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production rollback and filler removal preserve external punctuation', { timeout: 180_000 }, async () => {
  // Independent default-30 worker; all prior production cases stay unchanged.
  await runBasicCases([
    [true, 'Did you visit example.com? change this scratch that tomorrow', 'Did you visit example.com? Tomorrow'],
    [true, 'Visit https://example.com! change this scratch that tomorrow', 'Visit https://example.com! Tomorrow'],
    [true, 'Visit localhost:3000? change this scratch that tomorrow', 'Visit localhost:3000? Tomorrow'],
    [true, 'Visit example.com?query=value scratch that tomorrow', 'Tomorrow'],
    [true, 'Hello. visit example.com?query=value scratch that tomorrow', 'Hello. Tomorrow'],
    [false, 'Did you visit example.com? change this scratch that tomorrow', 'Did you visit example.com? change this scratch that tomorrow'],
    [true, '"Did you visit example.com? change this scratch that tomorrow"', '"Did you visit example.com? change this scratch that tomorrow"'],
    [false, 'Please (um), continue', 'Please, continue'],
    [false, 'Please [um]; continue', 'Please; continue'],
    [false, 'Please {um}: continue', 'Please: continue'],
    [false, 'Please ([um]), continue', 'Please, continue'],
    [false, 'Please (um). continue', 'Please. Continue'],
    [false, 'Please (um)! continue', 'Please! Continue'],
    [false, 'Please (um)? continue', 'Please? Continue'],
    [false, 'Please, um, continue', 'Please, continue'],
    [true, 'Please "(um)," continue', 'Please "(um)," continue'],
    [true, 'Please `(um),` continue', 'Please `(um),` continue'],
  ]);
});
