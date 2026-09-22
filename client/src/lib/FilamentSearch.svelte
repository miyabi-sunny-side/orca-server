<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import { request, type Filament } from "./api";
  let {
    choose,
    close,
    disabled = false,
    clearDisabled = false,
    cancelDisabled = false,
    clearLabel,
    search = (q: string, signal: AbortSignal) =>
      request<Filament[]>(`/api/filaments?q=${encodeURIComponent(q)}`, {
        signal,
      }),
    filters,
    context,
    empty,
  }: {
    choose: (filament: Filament | null) => void;
    close: () => void;
    disabled?: boolean;
    clearDisabled?: boolean;
    cancelDisabled?: boolean;
    clearLabel?: string;
    search?: (q: string, signal: AbortSignal) => Promise<Filament[]>;
    filters?: Snippet;
    context?: Snippet<[() => void]>;
    empty?: Snippet;
  } = $props();
  let retry = $state(0);
  let query = $state(""),
    options = $state<Filament[]>([]),
    loading = $state(true),
    error = $state("");
  let input = $state<HTMLInputElement>(),
    list = $state<HTMLUListElement>();
  onMount(() => input?.focus());
  $effect(() => {
    void retry;
    const q = query,
      load = search,
      controller = new AbortController();
    loading = true;
    error = "";
    const timer = setTimeout(async () => {
      try {
        const result = await load(q, controller.signal);
        if (!controller.signal.aborted) options = result;
      } catch (cause) {
        if (!controller.signal.aborted) error = (cause as Error).message;
      } finally {
        if (!controller.signal.aborted) loading = false;
      }
    }, 150);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  });
  function move(event: KeyboardEvent) {
    if (event.key === "Escape" && !cancelDisabled) {
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }
    if (!["ArrowDown", "ArrowUp"].includes(event.key)) return;
    event.preventDefault();
    const buttons = Array.from(
      list?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [],
    );
    const next =
      buttons.indexOf(event.target as HTMLButtonElement) +
      (event.key === "ArrowDown" ? 1 : -1);
    if (next < 0) input?.focus();
    else buttons[next]?.focus();
  }
</script>

<div class="search-row">
  <input
    type="search"
    data-autofocus
    aria-label="材料を検索"
    placeholder="製品・メーカー・材質・色"
    bind:value={query}
    bind:this={input}
    onkeydown={move}
  />
  <button type="button" class="btn" disabled={cancelDisabled} onclick={close}
    >キャンセル</button
  >
</div>
{@render filters?.()}
{#if loading}<p class="caption" role="status">検索中…</p>{/if}
{#if error}<p role="alert">{error}</p>
  <button
    type="button"
    class="btn"
    onclick={() => {
      retry++;
    }}>検索をやり直す</button
  >{/if}
<ul class="choices" bind:this={list}>
  {#if clearLabel}<li>
      <button
        type="button"
        disabled={clearDisabled}
        onclick={() => choose(null)}
        onkeydown={move}>{clearLabel}</button
      >
    </li>{/if}
  {#if !loading && !error}{#each options as f (f.id)}<li>
        <button
          type="button"
          {disabled}
          data-filament-id={f.id}
          onclick={() => choose(f)}
          onkeydown={move}
        >
          <span class="swatch" style:background={`#${f.color}`}></span><span
            >{f.name}<small>{f.vendor} · {f.material}</small></span
          >
        </button>
      </li>{/each}{/if}
</ul>
{#if !loading && !error}
  {@render context?.(() => {
    retry++;
  })}
  {#if !options.length}
    {#if empty}{@render empty()}{:else}<p class="caption">
        一致する材料がありません。<a href="/filaments">材料を登録</a>
      </p>{/if}
  {/if}
{/if}

<style lang="sass">
  .search-row
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)
    input
      flex: 1 1 12ch
      min-width: 120px
      width: 100%
  .choices
    list-style: none
    margin: var(--sp-2) 0 0
    padding: 0
    button
      display: flex
      align-items: center
      gap: var(--sp-2)
      width: 100%
      min-height: 44px
      padding: var(--sp-2)
      color: var(--c-on-surface)
      background: var(--c-surface)
      border: 0
      border-bottom: 1px solid var(--c-border)
      text-align: left
      font: inherit
      overflow-wrap: anywhere
      cursor: pointer
      &:hover:not(:disabled)
        background: var(--c-hover-1)
      &:disabled
        opacity: .5
    small
      display: block
      color: var(--c-muted)
  .swatch
    width: 16px
    height: 16px
    flex: 0 0 16px
    border: 1px solid var(--c-muted)
    border-radius: 50%
  p
    margin: var(--sp-2) 0
</style>
