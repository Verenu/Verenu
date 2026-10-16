function normalizeCode(code: string): string {
  return code === 'OSLeft' ? 'MetaLeft' : code === 'OSRight' ? 'MetaRight' : code;
}

/** Capture the keys held together, committing on the first release. */
export class HotkeyCapture {
  private held = new Set<string>();

  press(code: string, repeat = false, modifiers: Partial<Record<'Control' | 'Alt' | 'Shift' | 'Meta', boolean>> = {}): string[] {
    code = normalizeCode(code);
    for (const modifier of ['Control', 'Alt', 'Shift', 'Meta'] as const) {
      if (modifiers[modifier] && ![...this.held].some((held) => held.startsWith(modifier))) {
        this.held.add(code.startsWith(modifier) ? code : `${modifier}Left`);
      }
    }
    if (!repeat && code) this.held.add(code);
    return [...this.held];
  }

  release(code: string): string[] | null {
    code = normalizeCode(code);
    const modifier = ['Control', 'Alt', 'Shift', 'Meta'].find((prefix) => code === `${prefix}Left` || code === `${prefix}Right`);
    if (!this.held.has(code) && !(modifier && [...this.held].some((held) => held.startsWith(modifier)))) return null;
    const chord = [...this.held];
    this.held.clear();
    return chord;
  }

  reset(): void {
    this.held.clear();
  }
}
