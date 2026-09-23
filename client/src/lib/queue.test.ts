import { estimateText } from "./queue";
import { menuReasons, type Job } from "./queue";
import { describe, expect, it } from "vitest";
import { slotLabel, printerText, type Printer } from "./queue";
const printer: Printer = {
  connection: "connected",
  synchronized: true,
  ready_to_print: true,
  print: { state: "IDLE", percent: null, remaining_minutes: null, error: 0 },
  ams: null,
};

it("keeps duplication available during the current job and removal limited to waiting", () => {
  const admission = { plate_version: 7, allowed: true, reason: null };
  for (const state of [
    "queued",
    "preparing",
    "printing",
    "awaiting_removal",
    "needs_attention",
  ] as const) {
    const reasons = menuReasons({ state, plate_deleted: false }, admission);
    expect(reasons.edit).toBe("");
    expect(reasons.duplicate).toBe("");
    expect(Boolean(reasons.remove)).toBe(state !== "queued");
  }
});

it("explains missing jobs, deleted plates and server admission without blocking waiting removal", () => {
  const job: Pick<Job, "state" | "plate_deleted"> = { state: "queued" };
  const missing = menuReasons(undefined, null);
  expect(Object.values(missing).every(Boolean)).toBe(true);
  const deleted = menuReasons({ ...job, plate_deleted: true }, null);
  expect(deleted.edit).toContain("削除");
  expect(deleted.duplicate).toContain("削除");
  expect(deleted.remove).toBe("");
  expect(menuReasons(job, null).duplicate).toContain("確認");
  for (const [reason, text] of [
    ["Queue holds at most 100 waiting jobs", "100件"],
    ["No confirmed AMS slot contains the plate material", "AMS"],
  ]) {
    expect(
      menuReasons(job, { plate_version: 7, allowed: false, reason }).duplicate,
    ).toContain(text);
  }
});
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

it("inserts dragged IDs before or after targets without moving on stale or identical positions", async () => {
  const { moveIndex } = await import("./queue");
  expect(moveIndex(["a", "b", "c", "d"], "a", "c", true)).toBe(2);
  expect(moveIndex(["a", "b", "c", "d"], "d", "b", false)).toBe(1);
  expect(moveIndex(["a", "b", "c"], "a", "b", false)).toBeNull();
  expect(moveIndex(["a", "b", "c"], "b", "b", true)).toBeNull();
  expect(moveIndex(["a", "b"], "missing", "b", true)).toBeNull();
  expect(moveIndex(["a", "b"], "a", "missing", false)).toBeNull();
});
it("summarizes current progress and waiting holds in one status line", async () => {
  const { jobStatus, failureMessage } = await import("./queue");
  const job = {
    state: "printing",
    estimate: { state: "ready", seconds: 4800, error: null },
  } as any;
  expect(
    jobStatus(job, {
      ...printer,
      print: { ...printer.print, percent: 35, remaining_minutes: 52 },
    }),
  ).toBe("印刷中 · 35% · 残り約52分");
  expect(
    jobStatus({ ...job, state: "queued", hold_reason: "no material" }),
  ).toBe("保留 · 約1時間20分");
  expect(
    failureMessage(
      "Selected build plate temperature is missing or zero for this material",
      { bed_type: "Cool Plate" } as any,
      "PETG-GF 黒",
    ),
  ).toContain("PETG-GF 黒");
  expect(
    failureMessage(
      "Selected build plate temperature is missing or zero for this material",
      { bed_type: "Cool Plate" } as any,
      "PETG-GF 黒",
    ),
  ).toContain("Cool Plate");
  expect(
    failureMessage(
      "Selected build plate temperature is missing or zero for this material",
      { bed_type: "Cool Plate" } as any,
      "PETG-GF 黒",
    ),
  ).toContain("0℃");
});
