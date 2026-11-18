import type { InterfaceFont } from './settings';

export const DEFAULT_INTERFACE_FONT: InterfaceFont = 'default';

export const INTERFACE_FONT_OPTIONS: { id: InterfaceFont; label: string; description: string }[] = [
  { id: 'default', label: 'Inter Tight', description: 'Verenu default' },
  { id: 'system', label: 'System font', description: 'Matches your desktop' },
  { id: 'mono', label: 'Mono', description: 'JetBrains Mono' },
];

export function normalizeInterfaceFont(value: unknown): InterfaceFont {
  return INTERFACE_FONT_OPTIONS.some((option) => option.id === value)
    ? (value as InterfaceFont)
    : DEFAULT_INTERFACE_FONT;
}

/** The default font needs no attribute; theme.css swaps `--sans` for the others. */
export function applyInterfaceFont(root: HTMLElement, font: InterfaceFont): void {
  if (font === DEFAULT_INTERFACE_FONT) delete root.dataset.font;
  else root.dataset.font = font;
}
