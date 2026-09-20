export type Selection = {
  machine?: string;
  process: string;
  filament: string;
  bed: string;
};
export type Plate = {
  id: string;
  revision: string;
  name: string;
  models: { name: string; path: string; source: string | null }[];
  settings: { slicer?: Selection; [key: string]: unknown };
  project: string | null;
  print: string | null;
};
export type Profiles = {
  version: string;
  printer: string;
  processes: string[];
  filaments: string[];
  beds: string[];
  defaults: Selection;
};
export type Layout = {
  revision: string;
  models: ModelBounds[];
  bed: [[number, number], [number, number]];
};
export type ModelBounds = { index: number; bounds: [number, number][] };

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

export async function request<T>(
  path: string,
  options?: RequestInit,
): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, options);
  } catch (error) {
    if (options?.signal?.aborted) throw error;
    throw new Error(
      "サーバーに接続できません。接続を確認して再試行してください。",
    );
  }
  if (response.ok)
    return response.status === 204 ? (undefined as T) : response.json();
  const data = await response.json().catch(() => ({}));
  if (path.split("?")[0] === "/api/queue") {
    const message =
      response.status === 409
        ? "プレート・キュー・プリンターの状態が変わりました。内容を確認して操作し直してください。"
        : "キューを操作できませんでした。プレートやプリンターの状態を確認してください。";
    throw new ApiError(response.status, message);
  }
  if (path.startsWith("/api/printers")) {
    const message: Record<number, string> = {
      400: "接続情報、証明書、機種と工程の組み合わせを確認してください。",
      404: "プリンターが見つかりません。一覧から開き直してください。",
      409: "機器が使用中、または同じシリアル番号が登録されています。印刷状態とキューを確認してください。",
      503: "プロファイルまたは保存先を利用できません。サーバーの設定を確認してください。",
    };
    throw new ApiError(
      response.status,
      message[response.status] ??
        "プリンター設定を保存できませんでした。再試行してください。",
    );
  }
  if (String(data.error).includes("fit together")) {
    throw new Error(
      "モデルが1枚のプレートに収まりません。選択するモデルを減らしてください。",
    );
  }
  const messages: Record<number, string> = {
    400: "入力やモデルを確認してください。配置できない形状の可能性もあります。",
    404: "プレートが見つかりません。一覧から開き直してください。",
    409: "別の処理が進行中、またはプレートが更新されました。読み直して再試行してください。",
    502: "モデルの取得またはスライスに失敗しました。接続やモデルを確認して再試行してください。",
    503:
      path.startsWith("/api/scad") || path.endsWith("/import")
        ? "モデルの取得先が未設定です。サーバーのscad-live接続設定を確認してください。"
        : "スライサーを利用できません。サーバーのOrcaSlicer設定を確認してください。",
    504: "処理が時間の上限に達しました。モデルを減らすか、時間設定を確認して再試行してください。",
  };
  throw new Error(
    messages[response.status] ??
      "処理に失敗しました。時間を置いて再試行してください。",
  );
}

export type Machine = { key: string; model: string; nozzle_diameter: string };
export type Printer = {
  id: string;
  name: string;
  host: string;
  serial: string;
  machine_profile_key: string;
  default_process_profile_key: string;
  bed_type: string;
  nozzle_material: string;
  mqtt_port: number;
  ftps_port: number;
  start_timeout_secs: number;
  machine: { model: string; nozzle_diameter: string } | null;
  configuration_error: string | null;
  status: {
    connection: string;
    nozzle_diameter: string | null;
    nozzle_material: string | null;
  };
};
