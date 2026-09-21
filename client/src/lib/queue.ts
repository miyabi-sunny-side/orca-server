export type Specification = {
  ams_slot_id: string;
  filament_id: string;
  required_machine_profile_key: string;
  process_profile_key: string;
  bed_type: string;
};
export type Job = {
  ams_slot_id: string | null;
  filament_id: string | null;
  required_machine_profile_key: string | null;
  process_profile_key: string | null;
  bed_type: string | null;
  actual_ams_slot?: number | null;
  plate_deleted?: boolean;
  id: string;
  plate_id: string;
  name: string;
  state:
    | "queued"
    | "preparing"
    | "printing"
    | "awaiting_removal"
    | "needs_attention";
  attempt_id: string | null;
  artifact_path: string | null;
  last_error: string | null;
  hold_reason?: string | null;
  estimate?: Estimate;
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
  epoch: string;
  generation: number;
  request_id: string;
  allowed: { next: boolean; retry: boolean; discard: boolean };
  waiting: Job[];
  current: Job | null;
  printer: Printer;
  admission?: {
    plate_version: number;
    allowed: boolean;
    reason: string | null;
  } | null;
};
export type Action =
  | { type: "add"; plate_id: string; plate_version: number }
  | { type: "move"; job_id: string; index: number }
  | { type: "remove"; job_id: string }
  | { type: "reestimate"; job_id: string }
  | {
      type: "next";
      expected_job: string;
      removed_job: string | null;
      cleared: true;
    }
  | { type: "retry" | "discard"; expected_job: string; cleared: true };
export type Command = {
  epoch: string;
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
  queued: "待機中",
  preparing: "準備中",
  printing: "印刷中",
  awaiting_removal: "完了 · 取り外し待ち",
  needs_attention: "確認が必要です",
};
export const failureText: Record<string, string> = {
  "Selected build plate temperature is missing or zero for this material":
    "選択したプレートの温度が未設定または0℃です。材料のベッド温度を設定してください。",
  "Plate has been deleted": "このプレートは一覧から削除されています。",
  "Queue holds at most 100 waiting jobs":
    "待機キューは100件までです。不要な待機分を削除してください。",
  "Complete the plate machine, material, process and bed conditions":
    "プレートの機種・材料・工程・ビルドプレートを設定してください。",
  "No confirmed AMS slot contains the plate material":
    "この実機のAMSに指定材料の装填を確認できません。AMSの材料を確認してください。",
  "Wait for a current printer report":
    "プリンターの接続・装填状態を確認中です。",
  "Configure this material for the required machine and nozzle first":
    "要求する機種・ノズル用の材料設定を登録してください。",

  "Wait for a current, ready printer report":
    "プリンターの同期・待機状態を確認中です。",
  "Required machine or nozzle differs from the registered configuration":
    "要求する機種・ノズルが登録値と異なります。機器またはプレート条件を確認してください。",
  "Selected AMS slot does not contain the planned material":
    "AMSの現在の材料が使用予定と異なるか、装填を確認できません。",
  "Selected AMS slot is not confirmed present":
    "AMSスロットの装填を確認できません。",
  "Server restarted; inspect the printer before another start":
    "再起動前の開始結果を確認できません。本体を確認してください。自動再送はしません。",
  "scad-live returned an unsuccessful response":
    "最新モデルを取得できません。SCADのモデルを確認してください。",
  "scad-live request failed":
    "SCADへ接続できません。古いデータでは印刷していません。",
  "AMS assignment or material changed during preparation":
    "準備中にAMS割当または材料が変わりました。",
  "Printer status or selected AMS changed during transfer; no start command was sent":
    "転送中に機器またはAMSの状態が変わりました。開始命令は送っていません。",

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

export type Estimate = {
  state: "pending" | "calculating" | "ready" | "failed";
  seconds: number | null;
  error: string | null;
};
export function estimateText(estimate?: Estimate): string {
  if (!estimate || estimate.state === "pending") return "試算待ち";
  if (estimate.state === "calculating") return "試算中…";
  if (estimate.state === "failed" || !estimate.seconds)
    return "試算できませんでした";
  const minutes = Math.ceil(estimate.seconds / 60);
  const hours = Math.floor(minutes / 60),
    rest = minutes % 60;
  return `約${hours ? `${hours}時間` : ""}${rest ? `${rest}分` : ""}`;
}

export function moveIndex(
  ids: string[],
  from: string,
  target: string,
  after: boolean,
): number | null {
  if (from === target || !ids.includes(from) || !ids.includes(target))
    return null;
  const index = ids.filter((id) => id !== from).indexOf(target) + Number(after);
  return index === ids.indexOf(from) ? null : index;
}
export function jobStatus(job: Job, printer?: Printer): string {
  if (job.state === "queued")
    return `${job.hold_reason ? "保留 · " : ""}${estimateText(job.estimate)}`;
  if (job.state === "printing" && printer?.synchronized) {
    const parts = ["印刷中"];
    if (printer.print.percent !== null) parts.push(`${printer.print.percent}%`);
    if (printer.print.remaining_minutes !== null)
      parts.push(`残り約${printer.print.remaining_minutes}分`);
    return parts.join(" · ");
  }
  return phaseText[job.state];
}
export function failureMessage(
  error: string,
  job?: Job,
  material?: string,
): string {
  if (
    error ===
    "Selected build plate temperature is missing or zero for this material"
  )
    return `${material ?? "選択材料"}の${job?.bed_type ?? "ビルドプレート"}温度が未設定または0℃です。材料の初層・通常のベッド温度を設定してください。`;
  return failureText[error] ?? error;
}
