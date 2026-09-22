<script lang="ts">
  import { onMount } from "svelte";
  import {
    request,
    ApiError,
    type PlateConditions,
    type DefaultSettings,
    type Printer,
    type Profiles,
    type FilamentSetting,
  } from "./api";
  import { machineChoices } from "./plate";
  import StrengthFields from "./StrengthFields.svelte";
  import PlateFilamentPicker from "./PlateFilamentPicker.svelte";
  let {
    value = $bindable(),
    defaults,
    defaultsReading,
    defaultsError,
    changed,
    legacy = false,
  }: {
    value: PlateConditions;
    defaults?: DefaultSettings;
    defaultsReading: boolean;
    defaultsError: string;
    changed: (key: keyof PlateConditions) => void;
    legacy?: boolean;
  } = $props();
  const reasons = {
    printer: "プリンターを登録すると初期値を使えます。",
    printer_selection: "初期値に使うプリンターを選んでください。",
    profiles: "プリンターの機種・ノズルとプロファイルを確認してください。",
    process: "この機種に対応する既定の工程を選んでください。",
    ams_sync:
      "AMSの現在の装填を確認できません。プリンターとの接続を確認してください。",
    material: "AMSに、この機種で使える割当済みの材料がありません。",
  };
  const missing = $derived(
    [
      value.required_machine_profile_key,
      value.filament_id,
      value.process_profile_key,
      value.bed_type,
    ].some((v) => v == null),
  );
  let printers = $state<Printer[]>([]),
    profiles = $state<Profiles>();
  let setting = $state<FilamentSetting>(),
    loading = $state(true),
    error = $state(""),
    conditionError = $state(""),
    reading = $state(false),
    materialFound = $state(false);
  const machines = $derived(machineChoices(printers));
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
  $effect(() => {
    const machine = value.required_machine_profile_key,
      material = value.filament_id;
    const read = new AbortController();
    profiles = undefined;
    setting = undefined;
    materialFound = false;
    conditionError = "";
    reading = !!machine;
    if (machine)
      void Promise.allSettled([
        request<Profiles>(
          `/api/slicer/profiles?machine=${encodeURIComponent(machine)}`,
          { signal: read.signal },
        ),
        material
          ? request<{ settings: FilamentSetting[] }>(
              `/api/filaments/${material}`,
              { signal: read.signal },
            ).catch((e) => {
              if (e instanceof ApiError && e.status === 404) return undefined;
              throw e;
            })
          : Promise.resolve(undefined),
      ])
        .then(([p, f]) => {
          if (!read.signal.aborted) {
            if (p.status === "fulfilled") profiles = p.value;
            else conditionError = (p.reason as Error).message;
            if (f.status === "fulfilled") {
              materialFound = !!f.value;
              setting = f.value?.settings.find(
                (s) => s.machine_profile_key === machine,
              );
            } else conditionError = (f.reason as Error).message;
          }
        })
        .finally(() => {
          if (!read.signal.aborted) reading = false;
        });
    return () => read.abort();
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
    </p>
    <p class="caption">
      未設定でも保存できます。印刷キューへ追加する前に不足項目を設定してください。
    </p>
  {:else}<p class="caption">
      必要な項目だけ変更できます。次回以降の初期値は<a href="/printers"
        >プリンター設定</a
      >で変更します。
    </p>{/if}
  <label class="field"
    ><span>要求する機種・ノズル</span><select
      bind:value={value.required_machine_profile_key}
      onchange={() => changed("required_machine_profile_key")}
      disabled={loading}
    >
      <option value={null}>未設定</option>
      {#if value.required_machine_profile_key && !machines.includes(value.required_machine_profile_key)}<option
          value={value.required_machine_profile_key}
          >{value.required_machine_profile_key}（登録機なし）</option
        >{/if}
      {#each machines as machine}<option value={machine}>{machine}</option
        >{/each}
    </select></label
  >
  {#if !loading && !machines.length}<p class="caption">
      <a href="/printers/new">プリンターを登録</a
      >すると、所持機の機種・ノズルを選べます。
    </p>{/if}
  <PlateFilamentPicker
    value={value.filament_id}
    machine={value.required_machine_profile_key}
    choose={(id) => {
      value.filament_id = id;
      changed("filament_id");
    }}
  />
  <label class="field"
    ><span>工程（品質）</span><select
      bind:value={value.process_profile_key}
      onchange={() => changed("process_profile_key")}
      disabled={reading}
    >
      <option value={null}>未設定</option>
      {#if value.process_profile_key && !profiles?.processes.includes(value.process_profile_key)}<option
          value={value.process_profile_key}
          >{value.process_profile_key}（組合せを確認）</option
        >{/if}
      {#each profiles?.processes ?? [] as process}<option value={process}
          >{process}</option
        >{/each}
    </select></label
  >
  <label class="field"
    ><span>ビルドプレート</span><select
      bind:value={value.bed_type}
      onchange={() => changed("bed_type")}
      disabled={reading}
    >
      <option value={null}>未設定</option>
      {#if value.bed_type && !profiles?.beds.includes(value.bed_type)}<option
          value={value.bed_type}>{value.bed_type}（組合せを確認）</option
        >{/if}
      {#each profiles?.beds ?? [] as bed}<option value={bed}>{bed}</option
        >{/each}
    </select></label
  >
  <details class="settings-details">
    <summary>詳細設定</summary>
    <StrengthFields
      bind:value
      patterns={defaults?.infill_patterns}
      machine={value.required_machine_profile_key}
      process={value.process_profile_key}
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
            value.support_interface_filament_id ??= value.filament_id;
          changed("support_enabled");
        }}
      />
      <span>サポートを使う</span>
    </label>
    {#if value.support_enabled}
      <PlateFilamentPicker
        label="接触面のフィラメント"
        clearLabel="主材料と同じにする"
        value={value.support_interface_filament_id ?? value.filament_id}
        machine={value.required_machine_profile_key}
        choose={(id) => {
          value.support_interface_filament_id = id;
          changed("support_interface_filament_id");
        }}
      />
      <p class="caption">
        本体とサポートの支柱には、上で選んだフィラメントを使います。
      </p>
    {/if}
  </details>
  {#if loading || reading}<p class="caption" role="status">
      印刷条件を確認しています…
    </p>{/if}
  {#if error || conditionError}<div class="notice">
      <p role="alert">{error || conditionError}</p>
      <button class="btn" type="button" onclick={() => void load()}
        >条件を読み直す</button
      >
    </div>{/if}
  {#if !reading && materialFound && value.required_machine_profile_key && value.filament_id && (!setting || setting.error)}<p
      class="notice"
    >
      この機種で使う材料設定を確認してください。<a
        href={`/filaments/${value.filament_id}`}>材料設定へ</a
      >
    </p>{/if}
  {#if !reading && profiles && value.process_profile_key && !profiles.processes.includes(value.process_profile_key)}<p
      class="notice"
    >
      工程が要求する機種・ノズルに対応していません。工程を選び直すか、未設定に戻してください。
    </p>{/if}
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
