<script lang="ts">
  import { onMount } from "svelte";
  import {
    request,
    defaultStartOptions,
    type PlateConditions,
    type DefaultSettings,
    type Printer,
    type MaterialRole,
  } from "./api";
  import { roleFields } from "./plate";
  import StrengthFields from "./StrengthFields.svelte";
  import PlateFilamentPicker from "./PlateFilamentPicker.svelte";
  let {
    value = $bindable(),
    defaults,
    defaultsReading,
    defaultsError,
    changed,
    legacy = false,
    roles = ["primary"],
  }: {
    value: PlateConditions;
    defaults?: DefaultSettings;
    defaultsReading: boolean;
    defaultsError: string;
    changed: (key: keyof PlateConditions) => void;
    legacy?: boolean;
    roles?: MaterialRole[];
  } = $props();
  const reasons = {
    printer: "プリンターを登録すると初期値を使えます。",
    printer_selection: "初期値に使うプリンターを選んでください。",
    profiles: "プリンターの機種・ノズルとプロファイルを確認してください。",
    ams_sync:
      "AMSの現在の装填を確認できません。プリンターとの接続を確認してください。",
    material: "AMSに、この機種で使える割当済みの材料がありません。",
  };
  const missing = $derived(
    roles.map((role) => value[roleFields[role]]).some((v) => v == null),
  );
  const mainMaterial = $derived(
    value[roleFields[roles[0] ?? "primary"]] ?? null,
  );
  let printers = $state<Printer[]>([]),
    loading = $state(true),
    error = $state("");
  // The printer owns the machine and process; the default printer's show inherited values.
  const preview = $derived(
    printers.find((p) => p.id === defaults?.default_printer_id) ?? printers[0],
  );
  const controller = new AbortController();
  async function load() {
    loading = true;
    error = "";
    try {
      const p = await request<Printer[]>("/api/printers", {
        signal: controller.signal,
      });
      if (!controller.signal.aborted) printers = p;
    } catch (e) {
      if (!controller.signal.aborted) error = (e as Error).message;
    } finally {
      if (!controller.signal.aborted) loading = false;
    }
  }
  onMount(() => {
    void load();
    return () => controller.abort();
  });
</script>

<fieldset class="conditions">
  <legend>印刷条件</legend>
  {#if defaultsReading}<p class="caption" role="status">
      初期値を読み込んでいます…
    </p>
  {:else if defaultsError}<p class="notice" role="alert">
      初期値を取得できませんでした。{defaultsError}
      <a href="/printers">プリンター設定へ</a>
    </p>
  {:else if missing}
    <p class="notice">
      {defaults?.reason
        ? reasons[defaults.reason]
        : "不足している印刷条件を選んでください。"}
      <a
        href={defaults?.default_printer_id &&
        (defaults.reason === "ams_sync" || defaults.reason === "material")
          ? `/printers/${defaults.default_printer_id}/ams`
          : "/printers"}>設定を確認</a
      >
    </p>{/if}
  {#each roles as role (role)}<PlateFilamentPicker
      label={roles.length === 1 && role === "primary"
        ? "フィラメント"
        : `${role}のフィラメント`}
      value={value[roleFields[role]] ?? null}
      choose={(id) => {
        value[roleFields[role]] = id;
        changed(roleFields[role]);
      }}
    />{/each}
  <details class="settings-details">
    <summary>詳細設定</summary>
    <StrengthFields
      bind:value
      patterns={defaults?.infill_patterns}
      machine={preview?.machine_profile_key}
      process={preview?.default_process_profile_key}
      {legacy}
      {changed}
    />
    <label class="brim-option">
      <input
        type="checkbox"
        bind:checked={value.brim_enabled}
        onchange={() => changed("brim_enabled")}
      />
      <span>ブリムを付ける</span>
    </label>
    <label class="brim-option">
      <input
        type="checkbox"
        bind:checked={value.support_enabled}
        onchange={(e) => {
          if (e.currentTarget.checked)
            value.support_interface_filament_id ??= mainMaterial;
          changed("support_enabled");
        }}
      />
      <span>サポートを使う</span>
    </label>
    {#if value.support_enabled}
      <PlateFilamentPicker
        label="接触面のフィラメント"
        clearLabel="主材料と同じにする"
        value={value.support_interface_filament_id ?? mainMaterial}
        choose={(id) => {
          value.support_interface_filament_id = id;
          changed("support_interface_filament_id");
        }}
      />
    {/if}
  </details>
  <details class="settings-details">
    <summary>印刷開始</summary>
    {#each [["bed_leveling", "ベッドレベリング"], ["flow_calibration", "フロー較正"], ["timelapse", "タイムラプス"], ["vibration_calibration", "振動補正"]] as const as [key, label] (key)}
      <label class="brim-option">
        <input
          type="checkbox"
          checked={(value.start_options ?? defaultStartOptions)[key]}
          onchange={(e) => {
            value.start_options = {
              ...(value.start_options ?? defaultStartOptions),
              [key]: e.currentTarget.checked,
            };
            changed("start_options");
          }}
        />
        <span>{label}</span>
      </label>
    {/each}
  </details>
  {#if loading}<p class="caption" role="status">
      印刷条件を確認しています…
    </p>{/if}
  {#if error}<div class="notice">
      <p role="alert">{error}</p>
      <button class="btn" type="button" onclick={() => void load()}
        >条件を読み直す</button
      >
    </div>{/if}
</fieldset>

<style lang="sass">
  .brim-option
    display: flex
    align-items: center
    gap: var(--sp-2)
    min-height: 44px
    margin-top: var(--sp-3)
    cursor: pointer
    input
      accent-color: var(--c-accent)

  .conditions
    border: 0
    border-top: 1px solid var(--c-border)
    padding: var(--sp-4) 0 0
    margin: var(--sp-4) 0
    min-width: 0
    legend
      font-weight: 600
</style>
