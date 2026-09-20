<script lang="ts">
  import { onMount } from "svelte";
  import { request, type Layout, type Plate } from "../lib/api";
  let { id }: { id: string } = $props();
  let plate = $state<Plate>();
  let layout = $state<Layout>();
  let loading = $state(true);
  let error = $state("");
  let layoutError = $state("");
  let busy = $state("");
  const controller = new AbortController();
  const canReimport = $derived(
    plate?.models.every((model) => !!model.source) ?? false,
  );

  async function loadLayout(saved: Plate) {
    layout = undefined;
    layoutError = "";
    if (!saved.project) return;
    try {
      const result = await request<Layout>(`/api/plates/${id}/layout`, {
        signal: controller.signal,
      });
      if (result.revision !== saved.revision)
        throw new Error("プレートが更新されました。読み直してください。");
      if (!controller.signal.aborted) layout = result;
    } catch (cause) {
      if (!controller.signal.aborted) layoutError = (cause as Error).message;
    }
  }
  async function load() {
    error = "";
    loading = true;
    try {
      const result = await request<Plate>(`/api/plates/${id}`, {
        signal: controller.signal,
      });
      if (!controller.signal.aborted) {
        plate = result;
        await loadLayout(result);
      }
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
    } finally {
      if (!controller.signal.aborted) loading = false;
    }
  }
  onMount(() => {
    void load();
    return () => controller.abort();
  });

  async function build(reimport: boolean) {
    if (!plate || busy) return;
    error = "";
    try {
      if (reimport) {
        busy = "元モデルを取り込んでいます…";
        plate = await request<Plate>("/api/plates/import", {
          method: "POST",
          signal: controller.signal,
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            plate_id: plate.id,
            name: plate.name,
            models: plate.models.map((model) => model.source),
            settings: plate.settings,
          }),
        });
        layout = undefined;
      }
      busy = "配置・スライス中…";
      plate = await request<Plate>(`/api/plates/${id}/slice`, {
        method: "POST",
        signal: controller.signal,
      });
      await loadLayout(plate);
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
    } finally {
      if (!controller.signal.aborted) busy = "";
    }
  }
</script>

<svelte:head
  ><title>{plate?.name ?? "プレート"} · OrcaServer</title></svelte:head
>
<section class="content" aria-label="プレート詳細">
  <a class="back-link" href="/">プレート一覧へ</a>
  {#if loading}
    <p class="state" role="status">
      <span class="spinner" aria-hidden="true"
      ></span>プレートを読み込んでいます…
    </p>
  {:else if plate}
    <div class="page-heading">
      <h1 class="plate-name">{plate.name}</h1>
      <span class="caption">{plate.print ? "配置済み" : "スライス未完了"}</span>
    </div>
    {#if plate.print}<div class="actions">
        <a class="btn primary" href={`/queue?plate=${plate.id}`}>印刷キューへ</a
        >
      </div>{/if}
    {#if busy}<p class="state" role="status">
        <span class="spinner" aria-hidden="true"></span>{busy}
      </p>{/if}
    {#if error}<div class="notice">
        <p role="alert">{error}</p>
        <button class="btn" disabled={!!busy} onclick={() => void load()}
          >読み直す</button
        >
      </div>{/if}
    {#if layout}
      <figure>
        <svg
          viewBox="0 0 256 256"
          role="img"
          aria-label="モデルの配置（上面、256mm四方）"
        >
          <rect class="bed" x=".5" y=".5" width="255" height="255" />
          {#each layout.models as model}
            <g>
              <rect
                class="model"
                x={model.bounds[0][0]}
                y={256 - model.bounds[1][1]}
                width={model.bounds[0][1] - model.bounds[0][0]}
                height={model.bounds[1][1] - model.bounds[1][0]}
              />
              <text
                x={(model.bounds[0][0] + model.bounds[0][1]) / 2}
                y={256 - (model.bounds[1][0] + model.bounds[1][1]) / 2}
                text-anchor="middle"
                dominant-baseline="central">{model.index + 1}</text
              >
            </g>
          {/each}
        </svg>
        <figcaption>配置（上面）· 外形の範囲を表示</figcaption>
      </figure>
    {:else if layoutError}
      <div class="notice">
        <p role="alert">配置図を読み込めませんでした。{layoutError}</p>
        <button class="btn" disabled={!!busy} onclick={() => void load()}
          >読み直す</button
        >
      </div>
    {:else if !busy && !plate.print}
      <p class="state">
        モデルは保存されています。配置・スライスを実行してください。
      </p>
    {/if}
    <ol class="model-details">
      {#each plate.models as model, index}
        <li>
          <span>{model.name}</span
          >{#each layout?.models.filter((item) => item.index === index) ?? [] as item}<span
              class="caption"
              >{item.bounds
                .map(([low, high]) => (high - low).toFixed(1))
                .join(" × ")} mm</span
            >{/each}
        </li>
      {/each}
    </ol>
    <details class="settings">
      <summary>印刷設定</summary>
      <dl>
        <dt>工程</dt>
        <dd>{plate.settings.slicer?.process ?? "既定値"}</dd>
        <dt>材料</dt>
        <dd>{plate.settings.slicer?.filament ?? "既定値"}</dd>
        <dt>プレート種類</dt>
        <dd>{plate.settings.slicer?.bed ?? "既定値"}</dd>
      </dl>
    </details>
    <div class="actions">
      {#if plate.print}<a
          class="btn"
          href={`/api/plates/${plate.id}/files/${plate.print}`}
          download={`${plate.name}.gcode.3mf`}>印刷データを取得</a
        >{/if}
      {#if plate.project}<a
          class="btn"
          href={`/api/plates/${plate.id}/files/${plate.project}`}
          download={`${plate.name}.3mf`}>編集用3MFを取得</a
        >{/if}
      {#if !plate.print}<button
          class="btn primary"
          disabled={!!busy}
          onclick={() => void build(false)}>配置・スライス</button
        >{/if}
    </div>
    {#if canReimport}
      <details class="refresh">
        <summary>元モデルを更新</summary>
        <p>
          scad-liveから取り込み直し、現在の設定で配置・スライスを作り直します。
        </p>
        <button class="btn" disabled={!!busy} onclick={() => void build(true)}
          >取り込み直して配置</button
        >
      </details>
    {/if}
  {:else}
    <div class="notice">
      <p role="alert">{error}</p>
      <button class="btn" onclick={() => void load()}>再試行</button>
    </div>
  {/if}
</section>

<style lang="sass">
  .back-link
    display: inline-block
    margin-bottom: var(--sp-3)
    font-size: var(--fs-sm)

  .plate-name
    overflow-wrap: anywhere
    min-width: 0

  .page-heading > .caption
    flex-shrink: 0

  figure
    margin: var(--sp-4) auto
    width: min(100%, 320px)

  svg
    display: block
    width: 100%
    height: auto

  .bed
    fill: var(--c-surface-raised)
    stroke: var(--c-border)

  .model
    fill: var(--c-accent-subtle)
    stroke: var(--c-accent)
    stroke-width: 1

  text
    fill: var(--c-on-surface)
    font-size: 12px

  figcaption
    margin-top: var(--sp-2)
    text-align: center
    color: var(--c-muted)
    font-size: var(--fs-xs)

  .model-details
    padding-left: var(--sp-5)

    li
      margin-bottom: var(--sp-2)
      overflow-wrap: anywhere

    span
      display: block

  details
    font-size: var(--fs-sm)

  summary
    cursor: pointer

  dl
    margin: var(--sp-3) 0
    overflow-wrap: anywhere

  dt
    color: var(--c-muted)
    font-size: var(--fs-xs)

  dd
    margin: 0 0 var(--sp-2)

  .refresh
    margin-top: var(--sp-5)
    padding-top: var(--sp-3)
    border-top: 1px solid var(--c-border)
</style>
