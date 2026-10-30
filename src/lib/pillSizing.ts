export function pillBarSizing(scale: number) {
  const snap = (px: number) => Math.round(px * scale) / scale;
  const width = snap(3);
  return { width, gap: snap(5) - width };
}
