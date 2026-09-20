export type Job = {
  id: string;
  plate_id: string;
  revision: string;
  name: string;
  ams_slot: number;
};
type Ams = {
  units: {
    id: number;
    trays: { id: number; present: boolean | null; material: string | null }[];
  }[];
};
export type Printer = {
  connection: string;
  synchronized: boolean;
  ready_to_print: boolean;
  print: {
    state: string | null;
    percent: number | null;
    remaining_minutes: number | null;
    error: number | null;
  };
  ams: Ams | null;
};
export type QueueState = {
  generation: number;
  request_id: string;
  allowed: { next: boolean; retry: boolean; discard: boolean };
  waiting: Job[];
  current: {
    job: Job;
    phase: "starting" | "printing" | "awaiting_removal" | "needs_attention";
    message: string | null;
  } | null;
  printer: Printer;
};
export type Action =
  | { type: "add"; plate_id: string; revision: string; ams_slot: number }
  | { type: "move"; job_id: string; index: number }
  | { type: "remove"; job_id: string }
  | { type: "next" | "retry" | "discard"; expected_job: string; cleared: true };
export type Command = {
  generation: number;
  request_id: string;
  action: Action;
};

export function slotLabel(slot: number, ams: Ams | null) {
  const unit = Math.floor(slot / 4),
    tray = slot % 4;
  const state = ams?.units
    .find((u) => u.id === unit)
    ?.trays.find((t) => t.id === tray);
  const material =
    state?.present === false
      ? "未装填"
      : state?.present === true
        ? (state.material ?? "材料不明")
        : "状態未確認";
  return `AMS ${unit + 1} / スロット ${tray + 1} · ${material}`;
}
export function printerText(printer: Printer) {
  if (printer.connection === "unconfigured") return "プリンター未設定";
  if (printer.connection !== "connected") return "プリンター未接続";
  if (!printer.synchronized) return "プリンターの状態を確認中";
  if (printer.print.error)
    return `プリンターエラー ${printer.print.error} · 本体を確認してください`;
  return printer.ready_to_print
    ? "印刷できます"
    : "プリンターの終了・復帰を待っています";
}
export const phaseText = {
  starting: "転送・開始確認中",
  printing: "印刷中",
  awaiting_removal: "完了 · 取り外し待ち",
  needs_attention: "確認が必要です",
};
export const failureText: Record<string, string> = {
  "Selected AMS tray is absent, unknown or has a different material":
    "選択したAMSの材料・装填状態が合いません。本体を確認してください。",
  "FTPS transfer failed or timed out; no start command was sent":
    "印刷データを転送できませんでした。印刷開始の命令は送っていません。",
  "Printer disconnected; check the printer before another start":
    "プリンターとの接続が切れました。印刷中か本体で確認してください。",
  "Start confirmation timed out; the command will not be resent":
    "開始を確認できませんでした。本体を確認してください。自動では再送しません。",
  "Printer rejected the start request":
    "プリンターが開始要求を拒否しました。本体を確認してください。",
  "Printer reported an error; inspect the printer":
    "プリンターがエラーを報告しました。本体を確認してください。",
  "Print stopped; inspect the printer":
    "印刷が停止・一時停止しました。本体を確認してください。",
  "Print ended without a completion report; inspect the printer":
    "完了報告なしに印刷が終了しました。本体を確認してください。",
};
