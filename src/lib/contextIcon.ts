/// <reference lib="es2022.intl" />

export type CustomContextIcon = {
  kind: 'emoji' | 'letters';
  text: string;
  background: string;
  foreground: string;
};

// Keep custom artwork in the existing icon field so backups and sync retain it.
const PREFIX = 'custom-icon:';
export const isHexColor = (value: string): boolean => /^#[0-9a-f]{6}$/i.test(value);
const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

const EMOJI_GRAPHEME = /\p{Emoji_Presentation}|\p{Extended_Pictographic}\uFE0F|\u20E3|^\p{Regional_Indicator}{2}$/u;
const graphemes = (value: string): string[] => Array.from(segmenter.segment(value), (part) => part.segment);

// One or two visible characters, whatever they are. Grapheme segmentation keeps
// ZWJ emoji, flags, keycaps and letters with combining marks in one piece.
export function iconText(value: string): string {
  return graphemes(value.replace(/\s+/g, '')).slice(0, 2).join('');
}

// Presentation follows the content: emoji-only artwork has no badge, anything
// else (letters, or letters mixed with emoji) gets the colored badge.
export function iconKindFor(text: string): CustomContextIcon['kind'] {
  const parts = graphemes(text);
  return parts.length > 0 && parts.every((part) => EMOJI_GRAPHEME.test(part)) ? 'emoji' : 'letters';
}

export function encodeContextIcon(icon: Omit<CustomContextIcon, 'kind'>): string | null {
  const text = iconText(icon.text);
  return text ? PREFIX + JSON.stringify({ ...icon, text, kind: iconKindFor(text) }) : null;
}

export function parseContextIcon(value: string | null): CustomContextIcon | null {
  if (!value?.startsWith(PREFIX)) return null;
  try {
    const icon = JSON.parse(value.slice(PREFIX.length));
    if ((icon.kind !== 'emoji' && icon.kind !== 'letters') || typeof icon.text !== 'string'
      || typeof icon.background !== 'string' || !isHexColor(icon.background)
      || typeof icon.foreground !== 'string' || !isHexColor(icon.foreground)
      || !icon.text || iconText(icon.text) !== icon.text) return null;
    return { kind: iconKindFor(icon.text), text: icon.text, background: icon.background, foreground: icon.foreground };
  } catch {
    return null;
  }
}
