import { describe, expect, it } from 'vitest';
import { formatAppLabel } from './helpers';

describe('history app labels', () => {
  const apps = [
    { exe: 't3code', name: 'T3 Code Nightly' },
    { exe: 'org.gnome.texteditor', name: 'Text Editor' },
    { exe: 'code.exe', name: 'Visual Studio Code' },
  ];

  it('resolves Wayland IDs to an installed short window class', () => {
    expect(formatAppLabel('com.t3tools.t3code', apps)).toBe('T3 Code Nightly');
  });

  it('prefers an exact ID over a short window class and preserves name casing', () => {
    expect(formatAppLabel(' ORG.GNOME.TEXTEDITOR ', apps)).toBe('Text Editor');
    expect(formatAppLabel('com.t3tools.t3code', [
      ...apps, { exe: 'com.t3tools.t3code', name: 'T3 Code' },
    ])).toBe('T3 Code');
    expect(formatAppLabel('code.exe', apps)).toBe('Visual Studio Code');
  });

  it('keeps a readable fallback when discovery fails or the app was removed', () => {
    expect(formatAppLabel('some_app.exe')).toBe('Some App');
    expect(formatAppLabel('com.example.missing', apps)).toBe('Com.Example.Missing');
    expect(formatAppLabel('t3code', [{ exe: 't3code', name: ' ' }])).toBe('T3code');
  });

  it('does not guess between ambiguous matches or strip executable extensions', () => {
    expect(formatAppLabel('com.example.editor', [
      { exe: 'editor', name: 'Editor One' },
      { exe: 'EDITOR', name: 'Editor Two' },
    ])).toBe('Com.Example.Editor');
    expect(formatAppLabel('example.exe', [{ exe: 'exe', name: 'Wrong App' }])).toBe('Example');
  });
});
