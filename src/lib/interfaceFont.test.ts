import { describe, expect, it } from 'vitest';
import interfaceFontRust from '../../src-tauri/src/data/store/mod.rs?raw';
import { INTERFACE_FONT_OPTIONS, applyInterfaceFont, normalizeInterfaceFont } from './interfaceFont';

describe('interface font', () => {
  it('offers exactly the fonts the backend accepts', () => {
    const declaration = interfaceFontRust.match(/pub const INTERFACE_FONT_OPTIONS:\s*&\[&str\]\s*=\s*&\[([^\]]+)\];/);
    expect(declaration, 'Rust INTERFACE_FONT_OPTIONS declaration').not.toBeNull();
    const fonts = [...(declaration?.[1] ?? '').matchAll(/"([^"]+)"/g)].map((match) => match[1]);
    expect(INTERFACE_FONT_OPTIONS.map((option) => option.id).sort()).toEqual(fonts.sort());
  });

  it('falls back to the default font for unknown or missing values', () => {
    expect(normalizeInterfaceFont('mono')).toBe('mono');
    expect(normalizeInterfaceFont(null)).toBe('default');
    expect(normalizeInterfaceFont('comic')).toBe('default');
  });

  it('only marks the root for non-default fonts', () => {
    const root = { dataset: {} as Record<string, string> } as unknown as HTMLElement;
    applyInterfaceFont(root, 'system');
    expect(root.dataset.font).toBe('system');
    applyInterfaceFont(root, 'default');
    expect(root.dataset.font).toBeUndefined();
  });
});
