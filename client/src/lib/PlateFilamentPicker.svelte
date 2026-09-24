<script lang="ts">
  import { tick } from "svelte";
  import { request, type Filament } from "./api";
  import FilamentSearch from "./FilamentSearch.svelte";
  import Modal from "./Modal.svelte";
  import Icon from "./Icon.svelte";

  type Candidates = {
    filaments: Filament[];
    selected: Filament | null;
    selected_state: "unset" | "missing" | "loaded" | "unloaded" | "unconfirmed";
    printers: {
      id: string;
      name: string;
      state: "current" | "unconfirmed" | "error";
      unassigned: boolean;
    }[];
  };
  let {
    value,
    machine,
    label = "フィラメント",
    clearLabel = "指定を解除",
    choose,
  }: {
    value: string | null;
    machine: string | null;
    label?: string;
    clearLabel?: string;
    choose: (id: string | null) => void;
  } = $props();
  let open = $state(false),
    includeUnloaded = $state(false),
    retry = $state(0);
  let selector = $state<HTMLButtonElement>();
  let result = $state<Candidates>(),
    remembered = $state<Filament>();
  let reading = $state(false),
    error = $state("");
  const selected = $derived(remembered?.id === value ? remembered : undefined);
  const name = $derived(
    !value
      ? "フィラメントを選択"
      : (selected?.name ??
          (reading
            ? "材料を確認中…"
            : error
              ? "材料を確認できません"
              : "登録が見つかりません")),
  );
  const status = $derived(
    value && !reading && !error && result?.selected?.id === value
      ? (
          { unloaded: "未装填", unconfirmed: "装填未確認" } as Record<
            string,
            string
          >
        )[result.selected_state]
      : "",
  );

  // Capture the current scope before awaiting; a cancelled response cannot replace it.
  const search = $derived.by(() => {
    const id = value,
      profile = machine,
      all = includeUnloaded;
    return async (q: string, signal: AbortSignal) => {
      const params = new URLSearchParams({ q, include_unloaded: String(all) });
      if (id) params.set("selected_id", id);
      if (profile) params.set("machine", profile);
      const next = await request<Candidates>(`/api/plate-filaments?${params}`, {
        signal,
      });
      if (!signal.aborted) {
        result = next;
        if (next.selected) remembered = next.selected;
        else remembered = undefined;
        error = "";
      }
      return next.filaments;
    };
  });
  $effect(() => {
    void retry;
    const load = search,
      showing = open;
    const controller = new AbortController();
    if (!showing) {
      reading = true;
      error = "";
      void load("", controller.signal)
        .catch(() => {
          if (!controller.signal.aborted)
            error = "材料を取得できませんでした。";
        })
        .finally(() => {
          if (!controller.signal.aborted) reading = false;
        });
    } else reading = false;
    return () => controller.abort();
  });
  async function close() {
    open = false;
    await tick();
    selector?.focus();
  }
  function select(f: Filament | null) {
    remembered = f ?? undefined;
    choose(f?.id ?? null);
    void close();
  }
</script>

<div class="material-field">
  <span class="material-label">{label}</span>
  <button
    type="button"
    class="btn material-selector"
    bind:this={selector}
    aria-label={`${label}: ${name}`}
    aria-haspopup="dialog"
    aria-expanded={open}
    onclick={() => {
      includeUnloaded = false;
      result = undefined;
      open = true;
    }}
  >
    {#if selected}<span class="swatch" style:background={`#${selected.color}`}
      ></span>{/if}
    <span
      >{name}{#if status}<small>{status}</small>{/if}</span
    >
    <span class="disclosure-icon" aria-hidden="true"
      ><Icon name="chevron-left" /></span
    >
  </button>
  {#if error}<p class="caption" role="alert">
      {error}
      <button type="button" class="btn" onclick={() => retry++}>読み直す</button
      >
    </p>{/if}
</div>

{#if open}
  <Modal title={`${label}を選択`} onclose={() => void close()}>
    <FilamentSearch {search} choose={select} close={() => void close()}>
      {#snippet filters()}
        <label class="all-option"
          ><input type="checkbox" bind:checked={includeUnloaded} /><span
            >所持していないフィラメントを選択する</span
          ></label
        >
      {/snippet}
      {#snippet context(reload)}
        {#if result}
          {#each result.printers.filter((p) => p.state !== "current" || p.unassigned) as p (p.id)}
            <p class="caption inventory-state">
              {p.name}: {p.state === "error"
                ? "装填情報を取得できません"
                : p.state === "unconfirmed"
                  ? "装填未確認"
                  : "未割当の材料があります"}。<a href={`/printers/${p.id}/ams`}
                >AMSの材料割当</a
              >
            </p>
          {/each}
          {#if result.printers.some((p) => p.state !== "current")}<button
              type="button"
              class="btn"
              onclick={reload}>装填情報を再確認</button
            >{/if}
          {#if !result.printers.length && !includeUnloaded}<p class="caption">
              {machine
                ? "この機種・ノズルのプリンターがありません。"
                : "プリンターがありません。"}<a href="/printers"
                >プリンター設定</a
              >
            </p>{/if}
        {/if}
      {/snippet}
      {#snippet empty()}
        <p class="caption">
          一致する材料がありません。{#if !includeUnloaded}未装填の材料はチェックを入れて検索できます。{:else}<a
              href="/filaments">材料を登録</a
            >{/if}
        </p>
      {/snippet}
    </FilamentSearch>
    <button type="button" class="btn clear-choice" onclick={() => select(null)}
      >{clearLabel}</button
    >
  </Modal>
{/if}

<style lang="sass">
  .clear-choice
    margin-top: var(--sp-3)
    min-height: 44px
  .material-field
    margin: var(--sp-2) 0 var(--sp-3)
    min-width: 0
    overflow-wrap: anywhere
  .material-label
    color: var(--c-muted)
    font-size: var(--fs-xs)
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
    font: inherit
    > span:last-child
      margin-left: auto
    small
      display: block
      color: var(--c-muted)
  .disclosure-icon
    transform: rotate(-90deg)
    flex-shrink: 0
  .swatch
    width: 16px
    height: 16px
    flex: 0 0 16px
    border: 1px solid var(--c-muted)
    border-radius: 50%
  .all-option
    display: flex
    align-items: center
    gap: var(--sp-2)
    min-height: 44px
    margin-top: var(--sp-2)
    overflow-wrap: anywhere
    cursor: pointer
    input
      accent-color: var(--c-accent)
      flex-shrink: 0
  .inventory-state
    overflow-wrap: anywhere
</style>
