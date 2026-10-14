import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production voice commands carry sentence state across protected spans', { timeout: 180_000 }, async () => {
  await runBasicCases([
    [true, 'say "done." tomorrow', 'say "done." Tomorrow'],
    [true, 'say `done!` tomorrow', 'say `done!` Tomorrow'],
    [true, "say 'done?' tomorrow", "say 'done?' Tomorrow"],
    [true, 'say "done" tomorrow', 'say "done" tomorrow'],
    [true, 'say "Dr." tomorrow', 'say "Dr." tomorrow'],
    [true, 'open report.md tomorrow', 'open report.md tomorrow'],
    [true, '"done." tomorrow', '"done." Tomorrow'],
    [true, 'say "done." um tomorrow', 'say "done." Tomorrow'],
    [false, 'say "done." tomorrow', 'say "done." tomorrow'],
    [true, 'say done. tomorrow', 'say done. Tomorrow'],
    [true, 'say "done." at sign maria tomorrow', 'say "done." @maria tomorrow'],
    [true, 'say "done." new line tomorrow', 'say "done."\nTomorrow'],
    [true, 'say "done." "scratch that" tomorrow', 'say "done." "scratch that" tomorrow'],
    [true, 'I use sentence name tomorrow', 'I use Verenu. Tomorrow'],
    [true, 'say "done". tomorrow', 'say "done". Tomorrow'],
    [true, 'say "done"! tomorrow', 'say "done"! Tomorrow'],
    [true, 'say "done"? tomorrow', 'say "done"? Tomorrow'],
    [true, 'say "done", tomorrow', 'say "done", tomorrow'],
    [true, 'I use app name. tomorrow', 'I use Verenu. Tomorrow'],
  ]);
});
