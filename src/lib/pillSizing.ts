import { BAR_MAX_H, BAR_MIN_H } from './pillVisualizer';

export function pillBarSizing(scale: number) {
  const snap = (px: number) => Math.round(px * scale) / scale;
  const width = snap(3);
  return { width, gap: snap(5) - width };
}

export function pillBarHeight(height: number, scale: number, width: number) {
  const rest = 2 * Math.ceil((width * scale) / 2) / scale;
  return rest + (height - BAR_MIN_H) * (BAR_MAX_H - rest) / (BAR_MAX_H - BAR_MIN_H);
}
