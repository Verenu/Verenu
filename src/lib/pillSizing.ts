export function pillBarSizing(scale: number) {
  const snap = (px: number) => Math.round(px * scale) / scale;
  const width = snap(3);
  return { width, gap: snap(5) - width };
}

export function snapBarHeight(height: number, scale: number, width: number) {
  const evenDevicePx = (px: number) => 2 * Math.round((px * scale) / 2);
  return Math.max(evenDevicePx(height), evenDevicePx(width)) / scale;
}
