import assert from 'node:assert/strict';

export const basicCases = [
  [false, 'um send send it new line tomorrow', 'Send it new line tomorrow'],
  [true, 'um send send it new line tomorrow', 'Send it\nTomorrow'],
  [true, 'First. Second. scratch that scratch that', ''],
  [true, 'First. Second. scratch that scratch that Third', 'Third'],
  [true, 'Hello new line new line tomorrow', 'Hello\n\nTomorrow'],
  [true, 'Hello new paragraph new paragraph tomorrow', 'Hello\n\n\n\nTomorrow'],
  [false, 'Scratch that scratch that', 'Scratch that'],
  [true, 'Keep this put a colon change this scratch that tomorrow', 'Keep this: Tomorrow'],
  [false, 'Keep this put a colon change this scratch that tomorrow', 'Keep this put a colon change this scratch that tomorrow'],
  [false, 'Duran Duran', 'Duran Duran'],
  [true, 'Bora Bora', 'Bora Bora'],
  [false, 'NASA NASA', 'NASA NASA'],
  [true, 'I I I think so', 'I think so'],
  [false, 'send send it', 'send it'],
  [true, 'New York New York', 'New York New York'],
  [true, 'colon payload discard scratch that tomorrow', 'Keep this: scratch that Tomorrow'],
  [true, 'greeting discard scratch that tomorrow', 'um scratch that new line Tomorrow'],
  [true, 'Keep it semicolon change this scratch that greeting', 'Keep it; um scratch that new line'],
  [true, 'um please (um) use new line', 'Please use New Line'],
  [true, 'use question mark and new paragraph', 'use QuestionMark and new paragraph'],
  [true, 'I use app name um every day', 'I use Verenu every day'],
  [true, 'I use sentence name um every day', 'I use Verenu. Every day'],
  [false, 'I ordered tea, no, tea is unavailable.', 'I ordered tea, no, tea is unavailable.'],
  [true, 'signoff.', 'Thanks!'],
  [false, 'Take me to the ER', 'Take me to the ER'],
  [true, 'I studied at UM', 'I studied at UM'],
  [false, 'Um, take me to the ER', 'Take me to the ER'],
  [false, 'I met the doctor, no, the doctor who called me was a nurse', 'I met the doctor, no, the doctor who called me was a nurse'],
  [true, 'Keep this semicolon change this scratch that', 'Keep this;'],
  [true, '`Keep this semicolon change this scratch that`', '`Keep this semicolon change this scratch that`'],
];
export const mentionCases = [
  [true, 'Hello full stop at sign maria tomorrow', 'Hello. @maria tomorrow'],
  [true, 'ping at sign maria. tomorrow', 'ping @maria. Tomorrow'],
  [true, 'Hello full stop at the rate maria tomorrow', 'Hello. @maria tomorrow'],
  [false, 'ping at sign maria. tomorrow', 'ping at sign maria. tomorrow'],
  [true, '"ping at sign maria. tomorrow"', '"ping at sign maria. tomorrow"'],
  [true, '`ping at sign maria. tomorrow`', '`ping at sign maria. tomorrow`'],
];
// Each original scenario executes exactly once across independent workers.
assert.equal(basicCases.length, 30);
assert.equal(mentionCases.length, 6);
assert.equal(new Set([...basicCases, ...mentionCases].map(([enabled, raw]) => JSON.stringify([enabled, raw]))).size, 36);
