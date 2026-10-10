import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production commands preserve embedded periods and Basic repairs sentence-initial stutters', { timeout: 180_000 }, async () => {
  // Independent default-30 worker: do not add admissions to Basic's existing30.
  await runBasicCases([
    [true, 'open report.md', 'open report.md'],
    [true, 'visit example.com', 'visit example.com'],
    [true, 'open /tmp/report.md', 'open /tmp/report.md'],
    [true, 'version 1.25', 'version 1.25'],
    [true, 'visit https://example.com/report.md', 'visit https://example.com/report.md'],
    [true, 'load config.json.value', 'load config.json.value'],
    [true, 'open report.md. tomorrow', 'open report.md. Tomorrow'],
    [true, 'hello full stop.tomorrow', 'hello. Tomorrow'],
    [true, 'hello full stop tomorrow', 'hello. Tomorrow'],
    [true, 'hello!tomorrow', 'hello!Tomorrow'],
    [true, 'ping at sign maria.md tomorrow', 'ping @maria.md tomorrow'],
    [true, 'ping at sign maria. tomorrow', 'ping @maria. Tomorrow'],
    [true, 'Okay. Go go now', 'Okay. Go now'],
    [false, 'Okay. Go go now', 'Okay. Go now'],
    [false, 'Okay! Send send it', 'Okay! Send it'],
    [false, 'Okay? Go go now', 'Okay? Go now'],
    [false, 'Okay\nGo go now', 'Okay\nGo now'],
    [true, 'Okay. Duran Duran', 'Okay. Duran Duran'],
    [true, 'Okay. Bora Bora', 'Okay. Bora Bora'],
    [true, 'Okay. NASA NASA', 'Okay. NASA NASA'],
    [true, 'Okay. NASA nasa', 'Okay. NASA nasa'],
    [true, 'Okay. iPhone iPhone', 'Okay. iPhone iPhone'],
    [false, 'report.Go go', 'report.Go go'],
    [true, '"Okay. Go go now"', '"Okay. Go go now"'],
    [true, '`visit example.com`', '`visit example.com`'],
    [true, '[[VERENU_CLIPBOARD_report.md]]', '[[VERENU_CLIPBOARD_report.md]]'],
  ]);
});
