<script lang="ts">
  import { request, type Strength } from "./api";
  let {
    value = $bindable(),
    patterns = [],
    machine,
    process,
    legacy = false,
    required = false,
    changed = () => {},
  }: {
    value: Strength;
    patterns?: string[];
    machine?: string | null;
    process?: string | null;
    legacy?: boolean;
    required?: boolean;
    changed?: (key: keyof Strength) => void;
  } = $props();
  let shells = $state<Record<string, string>>();
  let error = $state("");
  let reading = $state(false);
  const edited = new Set<keyof Strength>();
  const labels: Record<string, string> = {
    adaptivecubic: "アダプティブキュービック",
    crosshatch: "クロスハッチ",
    gyroid: "ジャイロイド",
  };
  function change(
    key: keyof Strength,
    input: HTMLInputElement | HTMLSelectElement,
  ) {
    edited.add(key);
    Object.assign(value, {
      [key]:
        input.value === ""
          ? null
          : key === "sparse_infill_pattern"
            ? input.value
            : Number(input.value),
    });
    changed(key);
  }
  $effect(() => {
    const parameters = {
      machine,
      process,
      sparse_infill_pattern: value.sparse_infill_pattern,
      sparse_infill_density: value.sparse_infill_density,
      wall_loops: value.wall_loops,
    };
    const read = new AbortController();
    shells = undefined;
    error = "";
    reading = !!machine && !!process;
    if (machine && process) {
      const query = new URLSearchParams();
      for (const [key, v] of Object.entries(parameters))
        if (v != null) query.set(key, String(v));
      void request<Record<string, string>>(`/api/slicer/process?${query}`, {
        signal: read.signal,
      })
        .then((result) => {
          if (read.signal.aborted) return;
          shells = result;
          if (legacy) {
            for (const key of [
              "sparse_infill_pattern",
              "sparse_infill_density",
              "wall_loops",
            ] as const) {
              if (value[key] == null && result[key] != null && !edited.has(key))
                Object.assign(value, {
                  [key]:
                    key === "sparse_infill_pattern"
                      ? result[key]
                      : Number.parseFloat(result[key]),
                });
            }
          }
        })
        .catch((cause) => {
          if (!read.signal.aborted) error = (cause as Error).message;
        })
        .finally(() => {
          if (!read.signal.aborted) reading = false;
        });
    }
    return () => read.abort();
  });
</script>

<label class="field"
  ><span>インフィル</span>
  <select
    {required}
    value={value.sparse_infill_pattern ?? ""}
    onchange={(event) => change("sparse_infill_pattern", event.currentTarget)}
  >
    <option value="">{required ? "選択してください" : "工程から継承"}</option>
    {#if value.sparse_infill_pattern && !patterns.includes(value.sparse_infill_pattern)}
      <option value={value.sparse_infill_pattern}
        >{labels[value.sparse_infill_pattern] ??
          value.sparse_infill_pattern}</option
      >
    {/if}
    {#each patterns as pattern}<option value={pattern}
        >{labels[pattern] ? `${labels[pattern]} (${pattern})` : pattern}</option
      >{/each}
  </select>
</label>
<div class="numbers">
  <label class="field"
    ><span>充填率（%）</span><input
      {required}
      type="number"
      min="0"
      max="100"
      step="any"
      placeholder="工程から継承"
      value={value.sparse_infill_density ?? ""}
      oninput={(event) => change("sparse_infill_density", event.currentTarget)}
    /></label
  >
  <label class="field"
    ><span>壁の枚数（周）</span><input
      {required}
      type="number"
      min="0"
      max="1000"
      step="1"
      placeholder="工程から継承"
      value={value.wall_loops ?? ""}
      oninput={(event) => change("wall_loops", event.currentTarget)}
    /></label
  >
</div>
{#if reading}<p class="caption" role="status">上面・底面を計算しています…</p>
{:else if shells}<p class="caption" role="status">
    上面{shells.top_shell_layers}層・底面{shells.bottom_shell_layers}層{shells.top_shell_thickness !=
    null
      ? `（最小厚さ：上面${shells.top_shell_thickness} mm・底面${shells.bottom_shell_thickness ?? 0} mm）`
      : ""}
  </p>
{:else if error}<p class="notice" role="alert">{error}</p>
{:else}<p class="caption">
    機種と工程を選ぶと上面・底面の層数を確認できます。
  </p>{/if}
<p class="caption">
  壁2周を基準に上面・底面も増減します。充填率100%では工程の内部ソリッド方式で充填します。
</p>

<style lang="sass">
  .numbers
    display: grid
    grid-template-columns: repeat(2, minmax(0, 1fr))
    gap: var(--sp-4)
    @media (max-width: 480px)
      grid-template-columns: 1fr
</style>
