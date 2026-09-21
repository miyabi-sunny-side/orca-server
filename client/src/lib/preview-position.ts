type Rect = { left: number; top: number; right: number; bottom: number };

export function previewPosition(
  anchor: Rect,
  viewportWidth: number,
  viewportHeight: number,
  controls: Rect[],
) {
  const obstacles = [...controls, anchor];
  for (const [width, height] of [
    [260, 210],
    [200, 160],
    [160, 120],
  ]) {
    if (width + 16 > viewportWidth || height + 16 > viewportHeight) continue;
    const xs = new Set([
      8,
      viewportWidth - width - 8,
      ...obstacles.flatMap((r) => [r.left - width - 8, r.right + 8]),
    ]);
    const ys = new Set([
      8,
      viewportHeight - height - 8,
      ...obstacles.flatMap((r) => [r.top - height - 8, r.bottom + 8]),
    ]);
    let best: {
      left: number;
      top: number;
      width: number;
      height: number;
    } | null = null;
    let distance = Infinity;
    for (const left of xs)
      for (const top of ys) {
        if (
          left < 8 ||
          top < 8 ||
          left + width > viewportWidth - 8 ||
          top + height > viewportHeight - 8
        )
          continue;
        if (
          obstacles.some(
            (r) =>
              left < r.right &&
              left + width > r.left &&
              top < r.bottom &&
              top + height > r.top,
          )
        )
          continue;
        const d = Math.hypot(
          left + width / 2 - (anchor.left + anchor.right) / 2,
          top + height / 2 - (anchor.top + anchor.bottom) / 2,
        );
        if (d < distance) {
          best = { left, top, width, height };
          distance = d;
        }
      }
    if (best) return best;
  }
  return null;
}
