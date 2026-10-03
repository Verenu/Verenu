import { describe, expect, it } from 'vitest';
import { HotkeyCapture } from './hotkeyCapture';

describe('hotkey capture', () => {
  it('collects the entire held chord without saving on keydown', () => {
    const capture = new HotkeyCapture();
    for (const code of ['ControlLeft', 'AltLeft', 'ShiftLeft', 'MetaLeft', 'KeyK']) {
      capture.press(code);
    }
    capture.press('KeyK', true);
    expect(capture.release('KeyK')).toEqual(['ControlLeft', 'AltLeft', 'ShiftLeft', 'MetaLeft', 'KeyK']);
    expect(capture.release('ControlLeft')).toBeNull();
  });

  it('commits on whichever captured key is released first', () => {
    const capture = new HotkeyCapture();
    capture.press('KeyA');
    capture.press('KeyB');
    capture.press('KeyC');
    expect(capture.release('Unidentified')).toBeNull();
    expect(capture.release('KeyA')).toEqual(['KeyA', 'KeyB', 'KeyC']);
  });

  it('supports single keys and clears cancelled captures', () => {
    const capture = new HotkeyCapture();
    capture.press('F5');
    expect(capture.release('F5')).toEqual(['F5']);
    capture.press('ControlLeft');
    capture.reset();
    expect(capture.release('ControlLeft')).toBeNull();
    capture.press('F8');
    expect(capture.release('F8')).toEqual(['F8']);
  });

  it('includes modifiers already held when capture starts', () => {
    const capture = new HotkeyCapture();
    expect(capture.press('KeyK', false, { Control: true, Alt: true, Shift: true, Meta: true }))
      .toEqual(['ControlLeft', 'AltLeft', 'ShiftLeft', 'MetaLeft', 'KeyK']);
    expect(capture.release('ControlRight')).toEqual(['ControlLeft', 'AltLeft', 'ShiftLeft', 'MetaLeft', 'KeyK']);
  });
});
