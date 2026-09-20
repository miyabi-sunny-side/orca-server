import { expect, it } from "vitest";
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
