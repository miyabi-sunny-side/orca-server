import { estimateText } from "./queue";
import { describe, expect, it } from "vitest";
import { slotLabel, printerText, type Printer } from "./queue";
const printer: Printer = {
  connection: "connected",
  synchronized: true,
  ready_to_print: true,
  print: { state: "IDLE", percent: null, remaining_minutes: null, error: 0 },
  ams: null,
};
it("uses physical AMS numbering and keeps empty/unknown material distinct", () => {
  const ams = {
    units: [
      {
        id: 1,
        trays: [
          { id: 2, present: true, material: "PLA" },
          { id: 3, present: false, material: null },
        ],
      },
    ],
  };
  expect(slotLabel(6, ams)).toBe("AMS 2 / スロット 3 · PLA");
  expect(slotLabel(7, ams)).toContain("未装填");
  expect(slotLabel(15, null)).toBe("AMS 4 / スロット 4 · 状態未確認");
});
it("does not present unsynchronized or failed printers as ready", () => {
  expect(printerText(printer)).toBe("印刷できます");
  expect(
    printerText({
      ...printer,
      connection: "unconfigured",
      ready_to_print: false,
    }),
  ).toContain("未設定");
  expect(
    printerText({
      ...printer,
      connection: "disconnected",
      ready_to_print: false,
    }),
  ).toContain("未接続");
  expect(
    printerText({
      ...printer,
      synchronized: false,
      ready_to_print: false,
    }),
  ).toContain("確認中");
  expect(
    printerText({
      ...printer,
      ready_to_print: false,
      print: { ...printer.print, error: 42 },
    }),
  ).toContain("42");
});

describe("queue estimates", () => {
  it("keeps pending and failed distinct from approximate elapsed time", () => {
    expect(estimateText()).toBe("試算待ち");
    expect(
      estimateText({ state: "calculating", seconds: null, error: null }),
    ).toBe("試算中…");
    expect(
      estimateText({ state: "failed", seconds: null, error: "upstream" }),
    ).toBe("試算できませんでした");
    for (const [seconds, expected] of [
      [1, "約1分"],
      [1140, "約19分"],
      [3600, "約1時間"],
      [4800, "約1時間20分"],
      [3601, "約1時間1分"],
    ] as const) {
      expect(estimateText({ state: "ready", seconds, error: null })).toBe(
        expected,
      );
    }
  });
});
