<script lang="ts">
  import { onMount } from "svelte";
  import {
    request,
    type PlateConditions,
    type Printer,
    type Filament,
    type Profiles,
    type FilamentSetting,
  } from "./api";
  import { machineChoices } from "./plate";
  let { value = $bindable() }: { value: PlateConditions } = $props();
  let printers = $state<Printer[]>([]),
    filaments = $state<Filament[]>([]),
    profiles = $state<Profiles>();
  let setting = $state<FilamentSetting>(),
    loading = $state(true),
    error = $state(""),
    conditionError = $state(""),
    reading = $state(false);
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
  <p class="caption">
    未設定でも保存できます。キューへ追加する前に4項目を設定してください。
  </p>
  <label class="field"
    ><span>要求する機種・ノズル</span><select
      bind:value={value.required_machine_profile_key}
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
  .conditions
    border: 0
    border-top: 1px solid var(--c-border)
    padding: var(--sp-4) 0 0
    margin: var(--sp-4) 0
    min-width: 0
    legend
      font-weight: 600
</style>
