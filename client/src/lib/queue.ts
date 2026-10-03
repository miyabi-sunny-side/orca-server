export type Specification = {
  ams_slot_id: string;
  filament_id: string;
  required_machine_profile_key: string;
  process_profile_key: string;
  bed_type: string;
};
/** Printer-reported failure saved with the attempt; the same data feeds Discord. */
export type Failure = {
  kind: "rejected" | "device_error" | "stopped";
  field?: string;
  code?: string;
  reason?: string;
  state?: string;
};
/** Where the printer takes filament from; the user chooses it per job. */
export type Feed = "ams" | "external";
export type Job = {
  feed?: Feed;
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
  failure?: Failure | null;
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
  external_spool?: { material: string | null } | null;
  live?: Live;
};
type Reading = number | null;
export type Live = {
  temperatures: {
    nozzle: Reading;
    nozzle_target: Reading;
    bed: Reading;
    bed_target: Reading;
    chamber: Reading;
  };
  layer: { current: Reading; total: Reading };
};
/** "ノズル 212/220℃ · ベッド 60/60℃ · 層 12/337"; targets and layers only when set. */
export function liveLine(live?: Live): string {
  if (!live) return "";
  const t = live.temperatures;
  const temperature = (label: string, now: Reading, target: Reading) =>
    now === null
      ? null
      : `${label} ${Math.round(now)}${target ? `/${Math.round(target)}` : ""}℃`;
  return [
    temperature("ノズル", t.nozzle, t.nozzle_target),
    temperature("ベッド", t.bed, t.bed_target),
    live.layer.total
      ? `層 ${live.layer.current ?? 0}/${live.layer.total}`
      : null,
  ]
    .filter(Boolean)
    .join(" · ");
}
const printerStates: Record<string, string> = {
  IDLE: "待機中",
  PREPARE: "準備中",
  RUNNING: "印刷中",
  PAUSE: "一時停止中",
  FINISH: "完了",
  FAILED: "停止",
};
/** The reported `gcode_state` in Japanese; unknown values stay as reported. */
export function printerStateText(state: string | null): string {
  return state === null ? "状態不明" : (printerStates[state] ?? state);
}
/** Which print controls the reported printer state allows. */
export function jobControls(state: string | null) {
  const active = state === "RUNNING" || state === "PREPARE";
  const paused = state === "PAUSE";
  return { pause: active, resume: paused, stop: active || paused };
}
export type QueueState = {
  epoch: string;
  generation: number;
  request_id: string;
  allowed: { next: boolean; retry: boolean; discard: boolean };
  recovery?: {
    retry_reason: string | null;
    discard_reason: string | null;
    next_reason?: string | null;
  };
  waiting: Job[];
  current: Job | null;
  printer: Printer;
  admission?: {
    feed?: Feed;
    plate_version: number;
    allowed: boolean;
    reason: string | null;
  } | null;
};
export type Action =
  | { type: "add"; plate_id: string; plate_version: number; feed?: Feed }
  | { type: "feed"; job_id: string; feed: Feed }
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

export function feedLabel(feed?: Feed) {
  return feed === "external" ? "外部スプール" : "AMS";
}
/** A new job uses the external spool when the printer reports no AMS unit; unknown stays AMS. */
export function defaultFeed(
  status: { ams?: { units: unknown[] } | null } | null,
): Feed {
  return status?.ams === undefined || status.ams?.units.length
    ? "ams"
    : "external";
}
export function menuReasons(
  job: Pick<Job, "state" | "plate_deleted"> | undefined,
  admission: QueueState["admission"],
) {
  if (!job) {
    const reason = "このジョブはキューにありません。";
    return { edit: reason, duplicate: reason, remove: reason, feed: reason };
  }
  const edit = job.plate_deleted ? "プレートは一覧から削除されています。" : "";
  const duplicate =
    edit ||
    (!admission
      ? "追加条件を確認しています…"
      : admission.allowed
        ? ""
        : failureMessage(admission.reason ?? "追加条件を確認してください。"));
  const remove =
    job.state === "queued"
      ? ""
      : job.state === "awaiting_removal" || job.state === "needs_attention"
        ? "現在のジョブは、造形物を取り外してから取り外し確認の操作で終了してください。"
        : `${phaseText[job.state]}のジョブは削除できません。`;
  const feed =
    job.state === "queued" || job.state === "needs_attention"
      ? ""
      : `${phaseText[job.state]}のジョブは給材元を変更できません。`;
  return { edit, duplicate, remove, feed };
}

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
    return `プリンターエラー ${errorCode(printer.print.error)} · 本体を確認してください`;
  return printer.ready_to_print
    ? "印刷できます"
    : "プリンターの終了・復帰を待っています";
}
/** BambuStudio's `%08X` code with a dash after four digits, as used for the saved failures. */
export function errorCode(value: number): string {
  const hex = value.toString(16).toUpperCase().padStart(8, "0");
  return `${hex.slice(0, 4)}-${hex.slice(4)}`;
}
const failureKinds: Record<
  Failure["kind"],
  { title: string; missing: string }
> = {
  rejected: {
    title: "印刷開始の拒否",
    missing: "開始要求の応答にerr_codeなし",
  },
  device_error: { title: "印刷エラー", missing: "print_errorなし" },
  stopped: {
    title: "印刷停止",
    missing: "エラーコード0のFAILED。本体での手動停止も同じ報告です",
  },
};
function failureCode(failure: Failure): string | null {
  return failure.field && failure.code
    ? `${failure.field} ${failure.code}`
    : null;
}
/** The same title and lines as the Discord failure message. */
export function failureLines(failure: Failure) {
  const kind = failureKinds[failure.kind];
  const lines = [
    `コード: ${failureCode(failure) ?? `コード未取得（${kind.missing}）`}`,
  ];
  if (failure.state) lines.push(`本体の状態: ${failure.state}`);
  if (failure.reason) lines.push(`理由: ${failure.reason}`);
  return { title: kind.title, lines };
}
/** A fresh device error takes precedence over an older saved attempt's failure. */
export function queueFailure(
  job: Job | null,
  printer: Printer,
): { current: boolean; failure: Failure } | null {
  const saved = job?.failure;
  if (
    printer.connection === "connected" &&
    printer.synchronized &&
    printer.print.error
  ) {
    return {
      current: true,
      failure: {
        kind: "device_error",
        field: "print_error",
        code: errorCode(printer.print.error),
        state: printer.print.state ?? undefined,
      },
    };
  }
  return saved ? { current: false, failure: saved } : null;
}

// BambuStudio da8b44ee: resources/hms/hms_en_094.json (03008010),
// src/slic3r/GUI/HMS.cpp get_hms_wiki_url; no device ID is sent to the help site.
export function failureHelp(code?: string) {
  const hex = code?.replaceAll("-", "").toUpperCase();
  if (!hex || !/^[0-9A-F]{8}$/.test(hex)) return null;
  return {
    description:
      hex === "03008010"
        ? "ホットエンド冷却ファンの回転異常です。"
        : "このエラーコードの意味は未確認です。公式情報を確認してください。",
    url: `https://e.bambulab.com/index.php?${new URLSearchParams({ e: hex, s: "device_hms", lang: "ja" })}`,
  };
}
/** BambuStudio HMS.cpp get_hms_wiki_url with the long `%08X%08X` code; no device ID is sent. */
export function hmsHelp(code: string): string | null {
  const hex = code.replaceAll("_", "").toUpperCase();
  if (!/^[0-9A-F]{16}$/.test(hex)) return null;
  return `https://e.bambulab.com/index.php?${new URLSearchParams({ e: hex, s: "device_hms", lang: "ja" })}`;
}
export const phaseText = {
  queued: "待機中",
  preparing: "準備中",
  printing: "印刷中",
  awaiting_removal: "完了 · 取り外し待ち",
  needs_attention: "確認が必要です",
};
export const failureText: Record<string, string> = {
  "Wait for a matching terminal report for the previous start":
    "前の開始結果が不明です。本体から対象の停止・終了報告を受け取るまでお待ちください。",
  "Wait for a fresh synchronized printer report":
    "本体の最新状態を取得するまでお待ちください。",
  "Clear the printer error before recovery":
    "本体のエラーを解消してから再操作してください。",
  "Printer is still printing or preparing":
    "本体は印刷・準備中です。停止を確認してください。",
  "Printer is paused; stop the print before recovery":
    "本体は一時停止中です。印刷を停止してから再操作してください。",
  "Stopped print is not confirmed by the current report":
    "本体の停止を確認できません。印刷状況を確認してください。",
  "Printer report does not match the recovery target":
    "本体のジョブが変わっています。印刷状況を確認してください。",
  "No confirmed AMS slot contains the support interface material":
    "接触面用のフィラメントをAMSに装填し、材料を割り当ててください。",
  "No confirmed AMS slot contains the selected material":
    "選択した材料の装填を確認できません。AMSの材料割当を確認してください。",
  "AMS assignment or material settings changed during preparation":
    "準備中にAMS割当か材料設定が変わりました。保存済みの印刷条件と装填を確認してください。",
  "Print material order or AMS mapping differs from the frozen execution":
    "印刷データの材料順とAMS割当が一致しません。プレート条件と材料設定を確認してください。",

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
    "印刷が停止しました。本体を確認してください。",
  "Print paused; resume or stop it on the printer":
    "印刷が一時停止しています。本体で再開するか停止してください。",
  "Print ended without a completion report; inspect the printer":
    "完了報告なしに印刷が終了しました。本体を確認してください。",
  "The external spool prints one material; use the AMS or one material for every role":
    "外部スプールで印刷できるのは1材料だけです。AMSを使うか、全ての役割を同じ材料にしてください。",
  "External spool reports a different material; change the spool or its setting on the printer":
    "本体の外部スプール設定が予定の材料と異なります。スプールか本体の設定を確認してください。",
  "Material settings changed during preparation":
    "準備中に材料設定が変わりました。材料設定を確認してください。",
  "Only a waiting or attention-needed job can change its feed; reload the queue":
    "給材元は待機中か要確認のジョブだけ変更できます。キューを確認してください。",
  "Printer reconnected idle without this print; check the plate before starting again":
    "本体は再接続後に待機中で、この印刷は動いていません。プレートを確認して再印刷か削除を選んでください。",
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
  if (job.state === "needs_attention" && job.failure)
    return `${failureKinds[job.failure.kind].title} · ${failureCode(job.failure) ?? "コード未取得"}`;
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
