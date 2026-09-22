<script lang="ts">
  import { onMount, tick } from "svelte";
  import {
    request,
    type PlateConditions,
    type DefaultSettings,
    type Printer,
    type Filament,
    type Profiles,
    type FilamentSetting,
  } from "./api";
  import { machineChoices } from "./plate";
  import StrengthFields from "./StrengthFields.svelte";
  import FilamentSearch from "./FilamentSearch.svelte";
  import Icon from "./Icon.svelte";
  let searching = $state(false),
    selector = $state<HTMLButtonElement>();
  async function closeSearch() {
    searching = false;
    await tick();
    selector?.focus();
  }
  function chooseInterface(f: Filament | null) {
    value.support_interface_filament_id = f?.id ?? value.filament_id;
    changed("support_interface_filament_id");
    void closeSearch();
  }
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
    filaments = $state<Filament[]>([]),
    profiles = $state<Profiles>();
  let setting = $state<FilamentSetting>(),
    loading = $state(true),
    error = $state(""),
    conditionError = $state(""),
    reading = $state(false);
  const interfaceMaterial = $derived(
    filaments.find(
      (f) =>
        f.id === (value.support_interface_filament_id ?? value.filament_id),
    ),
  );
  const machines = $derived(machineChoices(printers));
  const controller = new AbortController();
  async function load() {
    loading = true;
    error = "";
    try {
      const [p, f] = await Promise.all([
        request<Printer[]>("/api/printers", { signal: controller.signal }),
        request<Filament[]>("/api/filaments", { signal: controller.signal }),
      ]);
      if (!controller.signal.aborted) {
        printers = p;
        filaments = f;
      }
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
    conditionError = "";
    reading = !!machine;
    if (machine)
      void Promise.all([
        request<Profiles>(
          `/api/slicer/profiles?machine=${encodeURIComponent(machine)}`,
          { signal: read.signal },
        ),
        material
          ? request<{ settings: FilamentSetting[] }>(
              `/api/filaments/${material}`,
              { signal: read.signal },
            )
          : Promise.resolve(undefined),
      ])
        .then(([p, f]) => {
          if (!read.signal.aborted) {
            profiles = p;
            setting = f?.settings.find(
              (s) => s.machine_profile_key === machine,
            );
          }
        })
        .catch((e) => {
          if (!read.signal.aborted) conditionError = (e as Error).message;
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
  <label class="field"
    ><span>フィラメント</span><select
      bind:value={value.filament_id}
      onchange={() => changed("filament_id")}
      disabled={loading}
    >
      <option value={null}>未設定</option>
      {#if value.filament_id && !filaments.some((f) => f.id === value.filament_id)}<option
          value={value.filament_id}>登録材料を確認してください</option
        >{/if}
      {#each filaments as f}<option value={f.id}
          >{f.name} · {f.vendor} · {f.material}</option
        >{/each}
    </select></label
  >
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
          else searching = false;
          changed("support_enabled");
        }}
      />
      <span>サポートを使う</span>
    </label>
    {#if value.support_enabled}
      <div class="interface-field">
        <span class="interface-label">接触面のフィラメント</span>
        <button
          type="button"
          class="btn material-selector"
          bind:this={selector}
          aria-label={`接触面のフィラメント: ${interfaceMaterial?.name ?? "未設定"}`}
          aria-expanded={searching}
          onclick={() => (searching = true)}
        >
          {#if interfaceMaterial}<span
              class="swatch"
              style:background={`#${interfaceMaterial.color}`}
            ></span>{/if}
          <span
            >{interfaceMaterial?.name ??
              (value.support_interface_filament_id
                ? "登録材料を確認してください"
                : "主材料を選ぶと設定されます")}</span
          >
          <span class="disclosure-icon" aria-hidden="true"
            ><Icon name="chevron-left" /></span
          >
        </button>
        {#if searching}<FilamentSearch
            choose={chooseInterface}
            close={() => void closeSearch()}
          />{/if}
        <p class="caption">
          本体とサポートの支柱には、上で選んだフィラメントを使います。
        </p>
      </div>
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
  {#if !reading && value.required_machine_profile_key && value.filament_id && (!setting || setting.error)}<p
      class="notice"
    >
      この機種で使う材料設定を確認してください。<a
        href={`/filaments/${value.filament_id}`}>材料設定へ</a
      >
    </p>{/if}
  {#if !reading && value.process_profile_key && !profiles?.processes.includes(value.process_profile_key)}<p
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

  .interface-field
    margin: var(--sp-2) 0 var(--sp-3)
    overflow-wrap: anywhere
  .interface-label
    display: block
    margin-bottom: var(--sp-2)
  .material-selector
    display: flex
    align-items: center
    gap: var(--sp-2)
    width: 100%
    min-height: 44px
    text-align: left
    white-space: normal
    > span:last-child
      margin-left: auto
  .disclosure-icon
    transform: rotate(-90deg)
  .swatch
    width: 16px
    height: 16px
    flex: 0 0 16px
    border: 1px solid var(--c-muted)
    border-radius: 50%

  .conditions
    border: 0
    border-top: 1px solid var(--c-border)
    padding: var(--sp-4) 0 0
    margin: var(--sp-4) 0
    min-width: 0
    legend
      font-weight: 600
</style>
