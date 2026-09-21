import { expect, test } from "vitest";
import { previewPosition } from "./preview-position";

test("the preview fits the viewport and avoids form controls at either edge", () => {
  for (const width of [320, 900]) {
    const controls = [
      { left: 0, top: 0, right: width, bottom: 150 },
      { left: 8, top: 180, right: 40, bottom: 620 },
      { left: 8, top: 650, right: width - 8, bottom: 710 },
    ];
    for (const top of [170, 600]) {
      const position = previewPosition(
        { left: 48, top, right: width - 8, bottom: top + 20 },
        width,
        800,
        controls,
      );
      expect(position).not.toBeNull();
      const p = position!;
      expect(p.left).toBeGreaterThanOrEqual(8);
      expect(p.top).toBeGreaterThanOrEqual(8);
      expect(p.left + p.width).toBeLessThanOrEqual(width - 8);
      expect(p.top + p.height).toBeLessThanOrEqual(792);
      for (const c of controls) {
        expect(
          p.left + p.width <= c.left ||
            p.left >= c.right ||
            p.top + p.height <= c.top ||
            p.top >= c.bottom,
        ).toBe(true);
      }
    }
  }
  expect(
    previewPosition({ left: 0, top: 0, right: 20, bottom: 20 }, 320, 400, [
      { left: 0, top: 0, right: 320, bottom: 400 },
    ]),
  ).toBeNull();
});
