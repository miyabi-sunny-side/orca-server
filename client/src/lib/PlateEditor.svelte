<script lang="ts">
  import HoverPreview from "./HoverPreview.svelte";
  import PlateConditions from "./PlateConditions.svelte";
  import { emptyConditions, initialConditions } from "./plate";
  import { onMount } from "svelte";
  import {
    request,
    type Plate,
    type PlateConditions as Conditions,
    type DefaultSettings,
  } from "./api";
  let {
    initial,
    saved,
    cancel,
  }: { initial?: Plate; saved: (plate: Plate) => void; cancel?: () => void } =
    $props();
  type Item = {
    id?: string;
    name: string;
    source: string | null;
    quantity: number;
  };
  let root = $state<HTMLDivElement>();
  let conditions = $state({ ...emptyConditions });
  let defaults = $state<DefaultSettings>();
  let defaultsError = $state("");
  let defaultsReading = $state(true);
  const edited = new Set<keyof Conditions>();
  async function loadDefaults() {
    const query = conditions.required_machine_profile_key
      ? `?machine=${encodeURIComponent(conditions.required_machine_profile_key)}`
      : "";
    try {
      const result = await request<DefaultSettings>(
        `/api/default-settings${query}`,
        { signal: controller.signal },
      );
      if (!controller.signal.aborted) {
        conditions = initialConditions(conditions, result.conditions, edited);
        defaults = result;
      }
    } catch (e) {
      if (!controller.signal.aborted) defaultsError = (e as Error).message;
    } finally {
      if (!controller.signal.aborted) defaultsReading = false;
    }
  }
  let step = $state(1),
    name = $state(""),
    selected = $state<Item[]>([]);
  let query = $state(""),
    retry = $state(0),
    models = $state<string[]>([]);
  let loading = $state(true),
    loadError = $state(""),
    error = $state(""),
    busy = $state(false);
  let search = $state<HTMLInputElement>(),
    list = $state<HTMLUListElement>();
  const total = $derived(
    selected.reduce((sum, m) => sum + (m.quantity || 0), 0),
  );
  const controller = new AbortController();
  onMount(() => {
    if (initial) {
      conditions = { ...(initial.conditions ?? emptyConditions) };
      name = initial.name;
      selected = initial.models.map((m) => ({ ...m }));
      step = 2;
    }
    void loadDefaults();
    return () => controller.abort();
  });
  $effect(() => {
    const q = query.trim();
    retry;
    const read = new AbortController();
    loading = true;
    loadError = "";
    const timer = setTimeout(async () => {
      try {
        const value = await request<string[]>(
          `/api/scad/models?q=${encodeURIComponent(q)}`,
          { signal: read.signal },
        );
        if (!read.signal.aborted) models = value;
      } catch (e) {
        if (!read.signal.aborted) loadError = (e as Error).message;
      } finally {
        if (!read.signal.aborted) loading = false;
      }
    }, 150);
    return () => {
      clearTimeout(timer);
      read.abort();
    };
  });
  function move(event: KeyboardEvent) {
    if (!["ArrowUp", "ArrowDown"].includes(event.key)) return;
    event.preventDefault();
    const row = (event.currentTarget as HTMLElement).closest("li");
    const next =
      event.key === "ArrowDown"
        ? row?.nextElementSibling
        : row?.previousElementSibling;
    const input = next?.querySelector(
      "input:not(:disabled)",
    ) as HTMLInputElement | null;
    if (input) input.focus();
    else if (event.key === "ArrowUp") search?.focus();
  }
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy || !selected.length || total > 64) return;
    busy = true;
    error = "";
    try {
      const plate = await request<Plate>(
        initial ? `/api/plates/${initial.id}` : "/api/plates/import",
        {
          method: initial ? "PUT" : "POST",
          headers: { "Content-Type": "application/json" },
          signal: controller.signal,
          body: JSON.stringify({
            name: name.trim(),
            ...(initial ? { version: initial.version } : {}),
            models: selected,
            conditions,
          }),
        },
      );
      if (!controller.signal.aborted) saved(plate);
    } catch (e) {
      if (!controller.signal.aborted) error = (e as Error).message;
    } finally {
      if (!controller.signal.aborted) busy = false;
    }
  }
</script>

<div bind:this={root}>
  {#if step === 1}
    <div class="model-search">
      <div class="page-heading">
        <h2>STLを選択</h2>
        <button
          class="btn primary"
          disabled={!selected.length}
          onclick={() => (step = 2)}>構成を確認（{selected.length}）</button
        >
      </div>
      <label class="field"
        ><span>モデル名で検索</span><input
          type="search"
          bind:value={query}
          bind:this={search}
          onkeydown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              list?.querySelector("input")?.focus();
            }
          }}
        /></label
      >
    </div>
    {#if loading}<p class="state" role="status">モデルを読み込んでいます…</p>
    {:else if loadError}<div class="notice">
        <p role="alert">{loadError}</p>
        <button class="btn" onclick={() => retry++}>再試行</button>
      </div>
    {:else if !models.length}<p class="state">
        {query ? "一致するモデルがありません" : "モデルがありません"}
      </p>
    {:else}<ul class="plate-list" bind:this={list}>
        {#each models as model}<li>
            <label
              class="plate-row model-row"
              class:selected={selected.some((m) => m.source === model)}
            >
              <input
                type="checkbox"
                checked={selected.some((m) => m.source === model)}
                disabled={selected.length >= 64 &&
                  !selected.some((m) => m.source === model)}
                onchange={(e) =>
                  (selected = e.currentTarget.checked
                    ? [...selected, { name: model, source: model, quantity: 1 }]
                    : selected.filter((m) => m.source !== model))}
                onkeydown={move}
              /><span
                data-stl-preview={`/api/scad/model?path=${encodeURIComponent(model)}`}
                >{model}</span
              >
            </label>
          </li>{/each}
      </ul>{/if}
    {#if cancel}<div class="actions">
        <button class="btn" onclick={cancel}>編集をやめる</button>
      </div>{/if}
  {:else}
    <form onsubmit={save}>
      <fieldset disabled={busy}>
        <label class="field"
          ><span>プレート名</span><input
            bind:value={name}
            required
            maxlength="256"
            placeholder="例: 机の小物入れ"
          /></label
        >
        <ul class="plate-list">
          {#each selected as model, index}<li class="plate-row">
              <strong
                data-stl-preview={model.source
                  ? `/api/scad/model?path=${encodeURIComponent(model.source)}`
                  : initial && model.id
                    ? `/api/plates/${initial.id}/models/${model.id}`
                    : undefined}>{model.name}</strong
              >
              <div class="item-actions">
                <label class="field"
                  ><span>個数</span><input
                    type="number"
                    min="1"
                    max="64"
                    step="1"
                    required
                    bind:value={model.quantity}
                    aria-label={`${model.name} の個数`}
                  /></label
                >
                <button
                  class="btn"
                  type="button"
                  aria-label={`${model.name}を構成から外す`}
                  onclick={() =>
                    (selected = selected.filter((_, i) => i !== index))}
                  >外す</button
                >
              </div>
            </li>{/each}
        </ul>
        <p class="caption">
          合計 {total} 個 / 最大64個。SCADモデルは試算時と印刷開始時に最新データを取得します。
        </p>
        <PlateConditions
          bind:value={conditions}
          {defaults}
          {defaultsReading}
          {defaultsError}
          changed={(key) => edited.add(key)}
        />
      </fieldset>
      {#if error}<div class="notice">
          <p role="alert">{error}</p>
          {#if initial}<button class="btn" type="button" onclick={cancel}
              >保存済みの構成へ戻る</button
            >{/if}
        </div>{/if}
      {#if busy}<p role="status">保存しています…</p>{/if}
      <div class="actions">
        <button
          class="btn primary"
          type="submit"
          disabled={busy || defaultsReading || !selected.length || total > 64}
          >保存</button
        ><button
          class="btn"
          type="button"
          disabled={busy}
          onclick={() => (step = 1)}>モデル選択へ</button
        >{#if cancel}<button
            class="btn"
            type="button"
            disabled={busy}
            onclick={cancel}>編集をやめる</button
          >{/if}
      </div>
    </form>
  {/if}
</div>
{#key step}<HoverPreview {root} />{/key}

<style lang="sass">
  .model-search
    position: sticky
    top: var(--header-h)
    z-index: 5
    background: var(--c-surface)
    padding: var(--sp-2) 0
    border-bottom: 1px solid var(--c-border)
    .page-heading
      margin-top: 0
    .field
      margin-bottom: 0

  fieldset
    border: 0
    padding: 0
    margin: 0
    min-width: 0
  .model-row
    flex-direction: row
    align-items: center
    gap: var(--sp-3)
    min-height: 48px
    cursor: pointer
    &.selected
      background: var(--c-hover-1)
    input
      flex-shrink: 0
  .item-actions
    display: flex
    align-items: end
    gap: var(--sp-3)
    .field
      margin: 0
      width: 100px
</style>
