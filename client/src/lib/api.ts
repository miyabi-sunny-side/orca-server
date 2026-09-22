export type Selection = {
  machine?: string;
  process: string;
  filament: string;
  bed: string;
};
export type Strength = {
  sparse_infill_pattern?: string | null;
  sparse_infill_density?: number | null;
  wall_loops?: number | null;
};
export type PlateConditions = Strength & {
  brim_enabled?: boolean;
  support_enabled?: boolean;
  support_interface_filament_id?: string | null;
  required_machine_profile_key: string | null;
  filament_id: string | null;
  process_profile_key: string | null;
  bed_type: string | null;
};
export type DefaultSettings = {
  default_printer_id: string | null;
  conditions: PlateConditions;
  infill_patterns: string[];
  reason:
    | "printer"
    | "printer_selection"
    | "profiles"
    | "process"
    | "ams_sync"
    | "material"
    | null;
};
export type Plate = {
  conditions: PlateConditions;
  id: string;
  version: number;
  name: string;
  models: {
    id: string;
    name: string;
    source: string | null;
    quantity: number;
  }[];
};
export type Profiles = {
  version: string;
  printer: string;
  processes: string[];
  filaments: string[];
  beds: string[];
  defaults: Selection;
};
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
  if (
    path.startsWith("/api/filaments") ||
    path.startsWith("/api/filament-products") ||
    /^\/api\/printers\/[^/]+\/ams(?:\/|$)/.test(path)
  ) {
    const messages: Record<number, string> = {
      400: "材料の種別・色・温度と、機種に対応する基本プロファイルを確認してください。",
      404: "材料・設定・AMSスロットが見つかりません。一覧から開き直してください。",
      409: path.endsWith("/adopt")
        ? "共通情報・全機種の設定が一致しないか、この色の印刷が進行中です。設定と印刷状態を確認してください。"
        : path.includes("/ams")
          ? "AMSの観測状態が変わったか未確認です。状態を更新して材料を選び直してください。"
          : "AMS・印刷ジョブからの参照、または同じ機種の設定が存在します。割当・キュー・登録済み設定を確認してください。",
      422: "入力の形式を確認してください。温度は整数で指定します。",
      503: "材料の保存先またはプロファイルを利用できません。接続とサーバー設定を確認してください。",
    };
    throw new ApiError(
      response.status,
      messages[response.status] ??
        "材料を操作できませんでした。再試行してください。",
    );
  }
  if (
    path.startsWith("/api/printers") ||
    path.split("?")[0] === "/api/default-settings"
  ) {
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
  if (String(data.error).includes("Selected process has no valid brim width")) {
    throw new Error(
      "この工程には有効なブリム幅がありません。別の工程を選ぶか「ブリムを付ける」をOFFにしてください。",
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

export type Filament = {
  id: string;
  name: string;
  vendor: string;
  material: string;
  color: string;
  bambu_filament_id: string | null;
};
export type FilamentColor = { id: string; name: string; color: string };
export type FilamentProduct = Omit<Filament, "color"> & {
  colors: FilamentColor[];
  settings: Omit<FilamentSetting, "filament_id">[];
};
export type FilamentTemperatures = {
  nozzle_temperature_initial_layer: string | null;
  nozzle_temperature: string | null;
  required_nozzle_hrc: string | null;
};
export type FilamentSetting = {
  id: string;
  filament_id: string;
  machine_profile_key: string;
  base_profile_key: string;
  overrides_json: {
    nozzle_temperature_initial_layer?: number;
    nozzle_temperature?: number;
    bed_temperature_initial_layer?: number;
    bed_temperature?: number;
  };
  resolved?: FilamentTemperatures | null;
  error?: string | null;
};
export type FilamentProfile = { key: string; resolved: FilamentTemperatures };
export type AmsSlot = {
  id: string;
  ams_id: number;
  slot_index: number;
  filament_id: string | null;
  mapping_source: string;
  revision: number;
  load_order: number | null;
  priority_order: number;
  priority_group: { id: string; revision: number }[];
  backup_peers: number[] | null;
  current: boolean;
  detect_on_insert: boolean | null;
  detect_on_power_up: boolean | null;
  filament: Filament | null;
  setting: FilamentSetting | null;
  reported: {
    present: boolean | null;
    material: string | null;
    brand: string | null;
    color: string | null;
    profile_id: string | null;
    tag_uid: string | null;
    temperature_min: number | null;
    temperature_max: number | null;
    remaining_percent: number | null;
    last_seen_at: number | null;
  };
};
export type AmsInventory = {
  printer_id: string;
  current: boolean;
  slots: AmsSlot[];
  auto_refill: {
    supported: boolean | null;
    enabled: boolean | null;
    groups: number[] | null;
  };
};
