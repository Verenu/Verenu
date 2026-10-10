import { describe, expect, it } from 'vitest';
import { isT3App } from './t3Skills';

describe('T3 destination identities', () => {
  it.each(['t3code', 'com.t3tools.T3Code', 'T3 Code.app', 't3-code.exe', 't3-code-nightly-20261004.exe', '/opt/t3code', 'C:\\Apps\\t3-code.exe'])('recognizes %s', name => expect(isT3App(name)).toBe(true));
  it.each(['electron', 't3-code-notes.exe', 't3-code-nightly-other.exe', 'editor', 'chrome.exe'])('rejects %s', name => expect(isT3App(name)).toBe(false));
});
