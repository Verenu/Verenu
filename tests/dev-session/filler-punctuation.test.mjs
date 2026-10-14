import test from 'node:test';
import { runBasicCases } from './helpers/basic-cleanup.mjs';

test('production filler punctuation and comma rollback retain preceding speech', { timeout: 180_000 }, async () => {
  await runBasicCases([
    [true, 'Keep this comma change this scratch that tomorrow', 'Keep this, Tomorrow'],
    [true, 'Keep this, change this scratch that tomorrow', 'Keep this, Tomorrow'],
    [true, 'Keep this comma change this scratch that', 'Keep this,'],
    [true, 'visit https://example.com/a,b scratch that tomorrow', 'Tomorrow'],
    [true, 'Keep this um semicolon continue', 'Keep this; continue'],
    [true, 'Keep this um comma continue', 'Keep this, continue'],
    [true, 'Keep this um put a colon continue', 'Keep this: continue'],
    [true, 'Keep this um semicolon', 'Keep this;'],
    [true, 'Keep this um comma', 'Keep this,'],
    [true, 'Keep this um put a colon', 'Keep this:'],
    [true, 'um semicolon continue', 'Continue'],
    [true, 'um comma continue', 'Continue'],
    [true, 'um put a colon continue', 'Continue'],
    [true, 'Keep this, you know, continue', 'Keep this continue'],
    [true, 'send, um, it', 'send, it'],
    [false, 'Keep this um semicolon continue', 'Keep this semicolon continue'],
    [false, 'Keep this comma change this scratch that tomorrow', 'Keep this comma change this scratch that tomorrow'],
    [true, '"Keep this um semicolon continue"', '"Keep this um semicolon continue"'],
    [true, '`Keep this comma change this scratch that`', '`Keep this comma change this scratch that`'],
    [true, 'open comma.com and /tmp/um,report.md', 'open comma.com and /tmp/um,report.md'],
    [true, '"Please", um, continue', '"Please", continue'],
    [true, '"Please" um, continue', '"Please", continue'],
    [true, 'yes. um, tomorrow', 'yes. Tomorrow'],
    [true, 'Call Dr. um, Smith', 'Call Dr. Smith'],
    [true, 'yes\num, tomorrow', 'yes\nTomorrow'],
    [true, 'yes! um, tomorrow', 'yes! Tomorrow'],
    [true, '"yes". um, tomorrow', '"yes". Tomorrow'],
    [true, 'I use app name, um, every day', 'I use Verenu, every day'],
    [true, 'yes new line um comma new paragraph tomorrow', 'yes\n\n\nTomorrow'],
    [true, 'yes full stop um comma new paragraph tomorrow', 'yes.\n\nTomorrow'],
  ]);
});
