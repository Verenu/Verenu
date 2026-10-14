import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production abbreviations and decomposed possessives retain real speech boundaries', { timeout: 180_000 }, async () => {
  await runBasicCases([
    [true, 'use e.g. lowercase names', 'use e.g. lowercase names'],
    [true, 'use i.e. lowercase names', 'use i.e. lowercase names'],
    [true, 'ask Dr. smith tomorrow', 'ask Dr. smith tomorrow'],
    [true, 'call A. smith tomorrow', 'call A. smith tomorrow'],
    [true, 'use U.S. spelling', 'use U.S. spelling'],
    [true, 'Call Dr. Smith scratch that tomorrow', 'Tomorrow'],
    [true, 'Call A. Smith scratch that tomorrow', 'Tomorrow'],
    [true, 'Use e.g. names scratch that tomorrow', 'Tomorrow'],
    [true, 'Use U.S. spelling scratch that tomorrow', 'Tomorrow'],
    [true, 'Hello. Call Dr. Smith scratch that tomorrow', 'Hello. Tomorrow'],
    [true, 'Okay. go now', 'Okay. Go now'],
    [true, 'I. go now', 'I. Go now'],
    [true, 'use examples full stop go now', 'use examples. Go now'],
    [false, 'Call Dr. Smith scratch that tomorrow', 'Call Dr. Smith scratch that tomorrow'],
    [true, '"Call Dr. Smith scratch that tomorrow"', '"Call Dr. Smith scratch that tomorrow"'],
    [true, '`use e.g. lowercase names`', '`use e.g. lowercase names`'],
    [false, 'cafe\u0301\'s um menu', 'cafe\u0301\'s menu'],
    [true, 'cafe\u0301\'s new line menu', 'cafe\u0301\'s\nMenu'],
    [false, 'cafe\u0301\'s new line menu', 'cafe\u0301\'s new line menu'],
    [true, 'say \'cafe\u0301\'s um menu\' literally', 'say \'cafe\u0301\'s um menu\' literally'],
    [true, 'say \'cafe\u0301\'s um new line', 'say \'cafe\u0301\'s um new line'],
    [true, 'ask Dr. um smith tomorrow', 'ask Dr. smith tomorrow'],
    [true, 'ping at sign dr. tomorrow', 'ping @dr. Tomorrow'],
    [true, 'ping @dr. tomorrow', 'ping @dr. Tomorrow'],
    [true, '@dr. tomorrow', '@dr. Tomorrow'],
    [true, 'ping at sign dr. change this scratch that tomorrow', 'ping @dr. Tomorrow'],
  ]);
});
