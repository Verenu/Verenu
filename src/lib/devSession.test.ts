import { describe, expect, it } from 'vitest';
import { encodePcmWav, loadSessionFixture } from './devSession';

describe('browser PCM conversion', () => {
  it('rejects fixture loading outside an authenticated browser session', async () => {
    await expect(loadSessionFixture('synthetic.wav')).rejects.toThrow('No browser dev session');
  });
  it('writes mono PCM16 WAV headers and clamps capture samples', () => {
    const bytes = encodePcmWav(new Float32Array([-2, 0, 2]));
    const view = new DataView(bytes.buffer);
    expect(new TextDecoder().decode(bytes.subarray(0, 4))).toBe('RIFF');
    expect(view.getUint32(24, true)).toBe(16000);
    expect(view.getUint16(22, true)).toBe(1);
    expect(view.getUint32(40, true)).toBe(6);
    expect(view.getInt16(44, true)).toBe(-32768);
    expect(view.getInt16(48, true)).toBe(32767);
  });
  it('rejects nonfinite samples, wrong rates, and overlong recordings', () => {
    expect(() => encodePcmWav(new Float32Array([NaN]))).toThrow();
    expect(() => encodePcmWav(new Float32Array(1), 48000)).toThrow();
    expect(() => encodePcmWav(new Float32Array(120 * 16000 + 1))).toThrow();
  });
});
