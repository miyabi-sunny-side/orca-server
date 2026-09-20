<script lang="ts">
  import { onMount } from "svelte";
  import {
    request,
    type Plate,
    type Profiles,
    type Selection,
  } from "../lib/api";

  let step = $state(1);
  let query = $state("");
  let retry = $state(0);
  let models = $state<string[]>([]);
  let selected = $state<string[]>([]);
  let loading = $state(true);
  let modelError = $state("");
  let profileError = $state("");
  let profiles = $state<Profiles>();
  let selection = $state<Selection>({ process: "", filament: "", bed: "" });
  let name = $state("");
  let busy = $state("");
  let error = $state("");
  let created = $state<Plate>();
  let imported = $state("");
  let search = $state<HTMLInputElement>();
  let list = $state<HTMLUListElement>();
  const controller = new AbortController();

  async function loadProfiles() {
    profileError = "";
    try {
      const result = await request<Profiles>("/api/slicer/profiles", {
        signal: controller.signal,
      });
      if (!controller.signal.aborted) {
        profiles = result;
        selection = { ...result.defaults };
      }
    } catch (cause) {
      if (!controller.signal.aborted) profileError = (cause as Error).message;
    }
  }
  onMount(() => {
    void loadProfiles();
    return () => controller.abort();
  });

  $effect(() => {
    const q = query.trim();
    retry;
    const read = new AbortController();
    loading = true;
    modelError = "";
    const timer = setTimeout(async () => {
      try {
        const result = await request<string[]>(
          `/api/scad/models?q=${encodeURIComponent(q)}`,
          { signal: read.signal },
        );
        if (!read.signal.aborted) {
          models = result;
          loading = false;
        }
      } catch (cause) {
        if (!read.signal.aborted) {
          modelError = (cause as Error).message;
          loading = false;
        }
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
    if (busy || !profiles || selected.length === 0) return;
    error = "";
    try {
      const input = {
        name: name.trim(),
        models: selected,
        settings: { slicer: selection },
      };
      const snapshot = JSON.stringify(input);
      if (!created || imported !== snapshot) {
        busy = "モデルを取り込んでいます…";
        created = await request<Plate>("/api/plates/import", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          signal: controller.signal,
          body: JSON.stringify({ ...input, plate_id: created?.id }),
        });
        imported = snapshot;
      }
      busy = "配置・スライス中…";
      const plate = await request<Plate>(`/api/plates/${created.id}/slice`, {
        method: "POST",
        signal: controller.signal,
      });
      window.location.assign(`/plates/${plate.id}`);
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
    } finally {
      if (!controller.signal.aborted) busy = "";
    }
  }
</script>

<svelte:head><title>新規作成 · OrcaServer</title></svelte:head>
<section class="content" aria-label="新しいプレート">
  <div class="page-heading">
    <h1>{step === 1 ? "STLを選択" : "設定して保存"}</h1>
    {#if step === 1}
      <button
        class="btn primary"
        disabled={selected.length === 0 || !profiles}
        onclick={() => (step = 2)}>設定へ（{selected.length}）</button
      >
    {/if}
  </div>
  {#if profileError}
    <div class="notice">
      <p role="alert">{profileError}</p>
      <button class="btn" onclick={() => void loadProfiles()}
        >設定を再読込み</button
      >
    </div>
  {/if}
  {#if step === 1}
    <label class="field" for="model-search"
      ><span>モデル名で検索</span><input
        id="model-search"
        type="search"
        placeholder="例: box、holder"
        bind:value={query}
        bind:this={search}
        onkeydown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            list?.querySelector("input")?.focus();
          }
        }}
      /></label
    >
    {#if loading}
      <p class="state" role="status">
        <span class="spinner" aria-hidden="true"
        ></span>モデルを読み込んでいます…
      </p>
    {:else if modelError}
      <div class="notice">
        <p role="alert">{modelError}</p>
        <button class="btn" onclick={() => retry++}>再試行</button>
      </div>
    {:else if models.length === 0}
      <p class="state">
        {query.trim() ? "一致するモデルがありません" : "モデルがありません"}
      </p>
    {:else}
      <ul class="plate-list" bind:this={list}>
        {#each models as model}
          <li>
            <label
              class="plate-row model-row"
              class:selected={selected.includes(model)}
            >
              <input
                type="checkbox"
                value={model}
                checked={selected.includes(model)}
                disabled={selected.length >= 64 && !selected.includes(model)}
                onchange={(event) =>
                  (selected = event.currentTarget.checked
                    ? [...selected, model]
                    : selected.filter((path) => path !== model))}
                onkeydown={move}
              />
              <span>{model}</span>
            </label>
          </li>
        {/each}
      </ul>
    {/if}
    <div class="actions"><a href="/">プレート一覧へ</a></div>
  {:else}
    <form onsubmit={save}>
      <fieldset disabled={!!busy}>
        <label class="field" for="plate-name"
          ><span>プレート名</span><input
            id="plate-name"
            type="text"
            bind:value={name}
            required
            placeholder="例: 机の小物入れ"
          /></label
        >
        <p class="caption">{profiles?.printer}</p>
        <label class="field" for="process"
          ><span>工程</span><select id="process" bind:value={selection.process}
            >{#each profiles?.processes ?? [] as value}<option>{value}</option
              >{/each}</select
          ></label
        >
        <label class="field" for="filament"
          ><span>材料</span><select
            id="filament"
            bind:value={selection.filament}
            >{#each profiles?.filaments ?? [] as value}<option>{value}</option
              >{/each}</select
          ></label
        >
        <label class="field" for="bed"
          ><span>プレート種類</span><select id="bed" bind:value={selection.bed}
            >{#each profiles?.beds ?? [] as value}<option>{value}</option
              >{/each}</select
          ></label
        >
        <details>
          <summary>選択したモデル（{selected.length}）</summary>
          <ul class="model-names">
            {#each selected as model}<li>{model}</li>{/each}
          </ul>
        </details>
      </fieldset>
      {#if busy}<p class="state" role="status">
          <span class="spinner" aria-hidden="true"></span>{busy}
        </p>{/if}
      {#if error}
        <div class="notice">
          <p role="alert">{error}</p>
          {#if created}<a href={`/plates/${created.id}`}
              >取り込み済みプレートを開く</a
            >{:else}<a href="/">保存済み一覧を確認</a>{/if}
        </div>
      {/if}
      <div class="actions">
        <button class="btn primary" type="submit" disabled={!!busy}
          >{created ? "もう一度配置する" : "配置して保存"}</button
        >
        <button
          class="btn"
          type="button"
          disabled={!!busy}
          onclick={() => (step = 1)}>選択へ戻る</button
        >
      </div>
    </form>
  {/if}
</section>

<style lang="sass">
  .model-row
    flex-direction: row
    align-items: center
    gap: var(--sp-3)
    min-height: 48px
    cursor: pointer

    input
      flex-shrink: 0

    &.selected
      background: var(--c-hover-1)

  fieldset
    min-width: 0
    border: 0
    padding: 0
    margin: 0

  .model-names
    padding-left: var(--sp-5)
    font-size: var(--fs-sm)
    overflow-wrap: anywhere

  summary
    cursor: pointer
    font-size: var(--fs-sm)
</style>
