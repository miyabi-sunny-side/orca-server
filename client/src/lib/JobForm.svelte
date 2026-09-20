<script lang="ts">
  import { onMount } from "svelte";
  import {
    request,
    type Printer,
    type Filament,
    type FilamentSetting,
    type Machine,
    type Profiles,
    type AmsInventory,
  } from "./api";
  import type { Specification } from "./queue";
  let {
    printer,
    initial,
    disabled = false,
    label,
    submit,
    cancel,
  }: {
    printer: Printer;
    initial?: Specification;
    disabled?: boolean;
    label: string;
    submit: (specification: Specification) => void;
    cancel?: () => void;
  } = $props();
  let filaments = $state<Filament[]>([]),
    machines = $state<Machine[]>([]),
    inventory = $state<AmsInventory>();
  let profiles = $state<Profiles>(),
    setting = $state<FilamentSetting>();
  let material = $state(""),
    slot = $state(""),
    machine = $state(""),
    process = $state(""),
    bed = $state("");
  let loading = $state(true),
    conditionsLoading = $state(false),
    error = $state(""),
    conditionError = $state("");
  let sequence = 0;
  const controller = new AbortController();
  const valid = $derived(
    !loading &&
      !conditionsLoading &&
      !error &&
      !conditionError &&
      !!setting &&
      !setting.error &&
      !!slot &&
      profiles?.processes.includes(process) &&
      profiles?.beds.includes(bed),
  );
  async function conditions(reset: boolean) {
    const ticket = ++sequence;
    conditionsLoading = true;
    conditionError = "";
    setting = undefined;
    try {
      const [p, f] = await Promise.all([
        request<Profiles>(
          `/api/slicer/profiles?machine=${encodeURIComponent(machine)}`,
          { signal: controller.signal },
        ),
        material
          ? request<{ settings: FilamentSetting[] }>(
              `/api/filaments/${material}`,
              { signal: controller.signal },
            )
          : Promise.resolve(undefined),
      ]);
      if (controller.signal.aborted || ticket !== sequence) return;
      profiles = p;
      setting = f?.settings.find((s) => s.machine_profile_key === machine);
      if (reset) {
        process =
          machine === printer.machine_profile_key
            ? printer.default_process_profile_key
            : p.defaults.process;
        bed = printer.bed_type;
      }
    } catch (e) {
      if (!controller.signal.aborted && ticket === sequence)
        conditionError = (e as Error).message;
    } finally {
      if (!controller.signal.aborted && ticket === sequence)
        conditionsLoading = false;
    }
  }
  async function load() {
    loading = true;
    error = "";
    try {
      const [f, m, i] = await Promise.all([
        request<Filament[]>("/api/filaments", { signal: controller.signal }),
        request<Machine[]>("/api/printers/profiles", {
          signal: controller.signal,
        }),
        request<AmsInventory>(`/api/printers/${printer.id}/ams`, {
          signal: controller.signal,
        }),
      ]);
      if (controller.signal.aborted) return;
      filaments = f;
      machines = m;
      inventory = i;
      await conditions(false);
    } catch (e) {
      if (!controller.signal.aborted) error = (e as Error).message;
    } finally {
      if (!controller.signal.aborted) loading = false;
    }
  }
  onMount(() => {
    material = initial?.filament_id ?? "";
    slot = initial?.ams_slot_id ?? "";
    machine =
      initial?.required_machine_profile_key ?? printer.machine_profile_key;
    process =
      initial?.process_profile_key ?? printer.default_process_profile_key;
    bed = initial?.bed_type ?? printer.bed_type;
    void load();
    return () => controller.abort();
  });
</script>

<form
  onsubmit={(e) => {
    e.preventDefault();
    if (valid && !disabled)
      submit({
        filament_id: material,
        ams_slot_id: slot,
        required_machine_profile_key: machine,
        process_profile_key: process,
        bed_type: bed,
      });
  }}
>
  <fieldset disabled={disabled || loading}>
    <label class="field"
      ><span>使用予定の材料</span><select
        required
        bind:value={material}
        onchange={() => void conditions(false)}
        ><option value="" disabled>材料を選択</option>
        {#each filaments as f}<option value={f.id}
            >{f.name} · {f.vendor} · {f.material} · {f.color}</option
          >{/each}
      </select></label
    >
    <label class="field"
      ><span>使用するAMSスロット</span><select required bind:value={slot}
        ><option value="" disabled>スロットを選択</option>
        {#each inventory?.slots ?? [] as s}<option value={s.id}
            >AMS {s.ams_id} / スロット {s.slot_index + 1} · 現在: {s.filament
              ?.name ?? "未割当"}{!inventory?.current
              ? "（未確認）"
              : ""}</option
          >{/each}
      </select></label
    >
    {#if !loading && !inventory?.slots.length}<p class="caption">
        AMSの観測情報がありません。<a href={`/printers/${printer.id}/ams`}
          >AMSを確認</a
        >
      </p>{/if}
    <label class="field"
      ><span>要求する機種・ノズル</span><select
        required
        bind:value={machine}
        onchange={() => void conditions(true)}
      >
        {#each machines as m}<option value={m.key}>{m.key}</option>{/each}
      </select></label
    >
    <label class="field"
      ><span>工程（品質）</span><select
        required
        bind:value={process}
        disabled={conditionsLoading}
      >
        {#each profiles?.processes ?? [] as p}<option>{p}</option>{/each}
      </select></label
    >
    <label class="field"
      ><span>ビルドプレート</span><select
        required
        bind:value={bed}
        disabled={conditionsLoading}
      >
        {#each profiles?.beds ?? [] as b}<option>{b}</option>{/each}
      </select></label
    >
  </fieldset>
  {#if loading || conditionsLoading}<p role="status" class="caption">
      材料と印刷条件を確認しています…
    </p>{/if}
  {#if error || conditionError}<div class="notice">
      <p role="alert">{error || conditionError}</p>
      <button class="btn" type="button" onclick={() => void load()}
        >条件を読み直す</button
      >
    </div>
  {:else if material && !conditionsLoading && (!setting || setting.error)}<p
      class="notice"
      role="alert"
    >
      この材料の機種・ノズル用設定がありません。<a
        href={`/filaments/${material}`}>材料設定を開く</a
      >
    </p>{/if}
  <p class="caption">
    開始時のAMS割当と要求ノズルが合うまで待機します。モデルとスライス設定は印刷準備の開始時に確定します。
  </p>
  <div class="actions">
    <button class="btn primary" type="submit" disabled={disabled || !valid}
      >{label}</button
    >{#if cancel}<button class="btn" type="button" {disabled} onclick={cancel}
        >編集をやめる</button
      >{/if}
  </div>
</form>

<style lang="sass">
  fieldset
    padding: 0
    border: 0
    margin: 0
    min-width: 0
</style>
