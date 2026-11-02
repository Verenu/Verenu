import { BAR_MAX_H, BAR_MIN_H } from './pillVisualizer';

export function pillBarSizing(scale: number, rasterScale = scale) {
  const snap = (px: number) => Math.round(px * scale) / scale;
  if (Math.abs(rasterScale - scale) > 0.001) {
    let bestPitch = 0;
    for (let backingPixels = 1; backingPixels <= 128; backingPixels++) {
      const pitch = backingPixels / rasterScale;
      const displayPixels = pitch * scale;
      if (Math.abs(displayPixels - Math.round(displayPixels)) > 0.0001) continue;
      if (displayPixels < 2) continue;
      if (!bestPitch || Math.abs(pitch - 5) < Math.abs(bestPitch - 5)) bestPitch = pitch;
    }
    if (bestPitch && Math.abs(bestPitch - 5) <= 1) {
      const displayPixels = Math.round(bestPitch * scale);
      const widthPixels = Math.min(displayPixels - 1, Math.max(1, Math.round(displayPixels * 0.6)));
      const width = widthPixels / scale;
      return { width, gap: bestPitch - width, sharedGrid: true };
    }
  }
  const width = snap(3);
  return { width, gap: snap(5) - width, sharedGrid: false };
}

export function pillBarHeight(height: number, scale: number, width: number) {
  const rest = 2 * Math.ceil((width * scale) / 2) / scale;
  return rest + (height - BAR_MIN_H) * (BAR_MAX_H - rest) / (BAR_MAX_H - BAR_MIN_H);
}
