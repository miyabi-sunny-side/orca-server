import { estimateText } from "./queue";
import {
  errorLine,
  feedLabel,
  hmsHelp,
  jobControls,
  liveLine,
  menuReasons,
  printerStateText,
  type Job,
} from "./queue";
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
      print: { ...printer.print, error: 0x03004001 },
    }),
  ).toBe("プリンターエラー 0300-4001 · 本体を確認してください");
});

it("keeps the saved failure's details for the expanded job and a short job status", async () => {
  const { failureLines, jobStatus } = await import("./queue");
  const failed = { state: "needs_attention" } as Job;
  expect(
    failureLines({
      kind: "device_error",
      field: "print_error",
      code: "0300-4001",
      state: "PAUSE",
    }),
  ).toEqual({
    title: "印刷エラー",
    lines: ["コード: print_error 0300-4001", "本体の状態: PAUSE"],
  });
  expect(
    jobStatus({
      ...failed,
      failure: {
        kind: "device_error",
        field: "print_error",
        code: "0300-4001",
      },
    }),
  ).toBe("要確認");
  expect(failureLines({ kind: "stopped", state: "FAILED" }).lines).toEqual([
    "コード: コード未取得（エラーコード0のFAILED。本体での手動停止も同じ報告です）",
    "本体の状態: FAILED",
  ]);
  expect(jobStatus({ ...failed, failure: { kind: "stopped" } })).toBe("要確認");
  expect(
    failureLines({ kind: "rejected", reason: "長い理由".repeat(40) }),
  ).toEqual({
    title: "印刷開始の拒否",
    lines: [
      "コード: コード未取得（開始要求の応答にerr_codeなし）",
      `理由: ${"長い理由".repeat(40)}`,
    ],
  });
  // The code is shown once, next to the current job, not again in its summary.
  expect(jobStatus({ ...failed, failure: null })).toBe("要確認");
});

it("keeps live errors distinct from saved failures and explains only verified codes", async () => {
  const { queueFailure, failureHelp } = await import("./queue");
  const saved = {
    kind: "device_error",
    field: "print_error",
    code: "0500-4003",
  } as const;
  const job = { state: "needs_attention", failure: saved } as Job;
  const paused = {
    ...printer,
    print: { ...printer.print, state: "PAUSE", error: 0x03008010 },
  };
  expect(queueFailure(job, paused)).toEqual({
    current: true,
    failure: {
      kind: "device_error",
      field: "print_error",
      code: "0300-8010",
      state: "PAUSE",
    },
  });
  expect(queueFailure(null, paused)?.failure.code).toBe("0300-8010");
  expect(queueFailure(job, { ...paused, synchronized: false })).toEqual({
    current: false,
    failure: saved,
  });
  expect(
    queueFailure(null, { ...paused, connection: "disconnected" }),
  ).toBeNull();
  expect(queueFailure(null, printer)).toBeNull();
  const rejected = {
    ...saved,
    kind: "rejected",
    field: "err_code",
    reason: "Storage full",
  } as const;
  expect(
    queueFailure(
      { ...job, failure: rejected },
      { ...paused, print: { ...paused.print, error: 0x05004003 } },
    ),
  ).toEqual({
    current: true,
    failure: {
      kind: "device_error",
      field: "print_error",
      code: "0500-4003",
      state: "PAUSE",
    },
  });
  expect(failureHelp("0300-8010")?.description).toContain(
    "ホットエンド冷却ファン",
  );
  expect(failureHelp("FFFF-1234")?.description).toContain("未確認");
  expect(failureHelp(undefined)).toBeNull();
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
    expect(
      estimateText({
        state: "failed",
        seconds: null,
        error: "Models must fit together on one plate",
        reason: "unfit",
      }),
    ).toBe("台に乗りません");
    expect(
      estimateText({
        state: "failed",
        seconds: null,
        error:
          "Configure this material for the required machine and nozzle first",
        reason: "material_setting",
      }),
    ).toBe("材料設定がありません");
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

it("labels the feed the printer uses", () => {
  expect(feedLabel("external")).toBe("外部スプール");
  expect(feedLabel(undefined)).toBe("AMS");
});

it("summarizes live temperatures and layers in one short line", () => {
  expect(
    liveLine({
      temperatures: {
        nozzle: 212.4,
        nozzle_target: 220,
        bed: 60,
        bed_target: 60,
        chamber: 31,
      },
      layer: { current: 12, total: 337 },
    }),
  ).toBe("ノズル 212/220℃ · ベッド 60/60℃ · 層 12/337");
  expect(
    liveLine({
      temperatures: {
        nozzle: 27,
        nozzle_target: 0,
        bed: 24,
        bed_target: 0,
        chamber: 5,
      },
      layer: { current: 0, total: 0 },
    }),
  ).toBe("ノズル 27℃ · ベッド 24℃");
  expect(liveLine(undefined)).toBe("");
  expect(
    liveLine({
      temperatures: {
        nozzle: null,
        nozzle_target: null,
        bed: null,
        bed_target: null,
        chamber: null,
      },
      layer: { current: null, total: null },
    }),
  ).toBe("");
});

it("offers pause or resume only for the matching printer state", () => {
  expect(jobControls("RUNNING")).toEqual({
    pause: true,
    resume: false,
    stop: true,
  });
  expect(jobControls("PREPARE")).toEqual({
    pause: true,
    resume: false,
    stop: true,
  });
  expect(jobControls("PAUSE")).toEqual({
    pause: false,
    resume: true,
    stop: true,
  });
  for (const state of ["IDLE", "FINISH", "FAILED", null])
    expect(jobControls(state)).toEqual({
      pause: false,
      resume: false,
      stop: false,
    });
});

it("links HMS codes to the official help without a device ID", () => {
  expect(hmsHelp("0300_0D00_0001_0004")).toBe(
    "https://e.bambulab.com/index.php?e=03000D0000010004&s=device_hms&lang=ja",
  );
  expect(hmsHelp("bad")).toBeNull();
});

it("names the printer's reported state in Japanese", () => {
  expect(printerStateText("RUNNING")).toBe("印刷中");
  expect(printerStateText("PAUSE")).toBe("一時停止中");
  expect(printerStateText("FAILED")).toBe("停止");
  expect(printerStateText("SLICING")).toBe("SLICING");
  expect(printerStateText(null)).toBe("状態不明");
});

it("shows a failure as one code with a short meaning and a help link", () => {
  const fan = errorLine({
    kind: "device_error",
    field: "print_error",
    code: "0300-8010",
  });
  expect(fan.text).toBe("0300-8010 ホットエンド冷却ファンの回転異常");
  expect(fan.help).toContain("e=03008010");
  const unknown = errorLine({
    kind: "device_error",
    field: "print_error",
    code: "0300-400C",
  });
  expect(unknown.text).toBe("0300-400C 意味は未確認");
  expect(errorLine({ kind: "stopped", state: "FAILED" })).toEqual({
    text: "印刷停止",
    help: null,
  });
  expect(errorLine({ kind: "rejected", reason: "Storage full" }).text).toBe(
    "開始拒否",
  );
});
