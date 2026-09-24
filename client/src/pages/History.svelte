<script lang="ts">
  import { onMount, tick } from "svelte";
  import { ApiError, request, type Plate } from "../lib/api";
  import { contextMenu } from "../lib/context-menu";
  import Modal from "../lib/Modal.svelte";
  import PlateQueueAdd from "../lib/PlateQueueAdd.svelte";

  type Entry = {
    id: number;
    plate_id: string;
    printer_id: string;
    name: string;
    completed_at: number;
    available: boolean;
  };
  type Page = { items: Entry[]; next_cursor: string | null };
  let items = $state<Entry[]>([]),
    cursor = $state<string | null>(null);
  let loading = $state(false),
    loaded = $state(false),
    error = $state("");
  let menu = $state<Entry>(),
    plate = $state<Plate>(),
    menuError = $state("");
  let reading = $state(false),
    unavailable = $state(false),
    queueBusy = $state(false);
  let addedTo = $state("");
  let menuRow: HTMLElement | undefined;
  const controller = new AbortController();
  const date = new Intl.DateTimeFormat("ja-JP", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  });
  async function load() {
    if (loading) return;
    loading = true;
    error = "";
    try {
      const value = await request<Page>(
        `/api/history${cursor ? `?before=${encodeURIComponent(cursor)}` : ""}`,
        { signal: controller.signal },
      );
      items = [...items, ...value.items];
      cursor = value.next_cursor;
      loaded = true;
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
    } finally {
      loading = false;
    }
  }
  async function readPlate() {
    if (!menu) return;
    const entry = menu;
    reading = true;
    menuError = "";
    try {
      const value = await request<Plate>(
        `/api/plates/${encodeURIComponent(entry.plate_id)}`,
        { signal: controller.signal },
      );
      if (menu === entry) plate = value;
    } catch (cause) {
      if (menu !== entry || controller.signal.aborted) return;
      if (cause instanceof ApiError && cause.status === 404) unavailable = true;
      else menuError = (cause as Error).message;
    } finally {
      if (menu === entry) reading = false;
    }
  }
  function open(entry: Entry, row: HTMLElement) {
    menu = entry;
    menuRow = row;
    plate = undefined;
    queueBusy = false;
    menuError = "";
    unavailable = !entry.available;
    reading = false;
    if (!unavailable) void readPlate();
  }
  async function close() {
    menu = undefined;
    plate = undefined;
    await tick();
    menuRow?.focus({ preventScroll: true });
  }
  onMount(() => {
    void load();
    return () => controller.abort();
  });
</script>

<svelte:head><title>プリント履歴 · OrcaServer</title></svelte:head>
<section class="content history" aria-label="プリント履歴">
  <div class="history-heading">
    <h1>プリント履歴</h1>
    <a href="/">印刷キューへ</a>
  </div>
  {#if addedTo}<p class="added" role="status">
      キューに追加しました。<a
        href={`/queue?printer_id=${encodeURIComponent(addedTo)}`}
        >キューを見る</a
      >
    </p>{/if}
  {#if items.length}<ul class="history-list">
      {#each items as entry (entry.id)}
        <li>
          <button
            class="history-row"
            type="button"
            aria-haspopup="dialog"
            use:contextMenu={(row) => open(entry, row)}
            onclick={(event) => open(entry, event.currentTarget)}
          >
            <strong>{entry.name}</strong><time
              datetime={new Date(entry.completed_at * 1000).toISOString()}
              >{date.format(entry.completed_at * 1000)}</time
            >
          </button>
        </li>
      {/each}
    </ul>{:else if loaded && !error && !loading}<p class="state">
      プリント履歴はまだありません
    </p>{/if}
  {#if loading}<p role="status">履歴を読み込んでいます…</p>{/if}
  {#if error}<div class="notice">
      <p role="alert">{error}</p>
      <button class="btn" onclick={() => void load()}>再試行</button>
    </div>
  {:else if cursor}<button
      class="btn older"
      disabled={loading}
      onclick={() => void load()}>以前の履歴を読み込む</button
    >{/if}
</section>

{#if menu}
  <Modal
    title={menu.name}
    onclose={() => void close()}
    dismissible={!queueBusy}
  >
    <div class="history-menu">
      {#if unavailable}<button class="btn primary" disabled>キューに追加</button
        >
        <p>プレートが削除されているため追加できません。</p>
      {:else if reading}<p role="status">プレートを読み込んでいます…</p>
      {:else if menuError}<p role="alert">{menuError}</p>
        <button class="btn" onclick={() => void readPlate()}>再試行</button>
      {:else if plate}<PlateQueueAdd
          bind:plate
          preferredPrinter={menu.printer_id}
          label="キューに追加"
          onbusy={(value) => (queueBusy = value)}
          onadded={(printerId) => {
            addedTo = printerId;
            void close();
          }}
        />{/if}
      {#if plate}<a
          class:blocked={queueBusy}
          aria-disabled={queueBusy}
          href={queueBusy ? undefined : `/plates/${plate.id}?edit=1`}
          >プレート編集</a
        >{/if}
    </div>
  </Modal>
{/if}

<style lang="sass">
  .history:has(.added)
    padding-bottom: calc(var(--sp-5) + 6em)
  .history-heading
    display: flex
    align-items: center
    justify-content: space-between
    flex-wrap: wrap
    gap: var(--sp-2)
    margin-bottom: var(--sp-3)
    h1
      margin: 0
      font-size: var(--fs-xl)
    a
      display: inline-flex
      align-items: center
      min-height: 44px
      font-size: var(--fs-sm)
  .history-list
    list-style: none
    margin: 0
    padding: 0
  .history-row
    display: flex
    align-items: center
    justify-content: space-between
    gap: var(--sp-3)
    width: 100%
    min-height: 60px
    padding: var(--sp-3) 0
    border: 0
    border-bottom: 1px solid var(--c-border)
    border-radius: 0
    background: transparent
    color: var(--c-on-surface)
    text-align: left
    cursor: pointer
    touch-action: pan-y
    -webkit-touch-callout: none
    user-select: none
    &:hover
      background: var(--c-hover-1)
    strong
      min-width: 0
      font-size: var(--fs-lg)
      overflow: hidden
      text-overflow: ellipsis
      white-space: nowrap
    time
      flex-shrink: 0
      color: var(--c-muted)
      font-size: var(--fs-sm)
      font-variant-numeric: tabular-nums
  .history-menu
    display: grid
    gap: var(--sp-3)
    overflow-wrap: anywhere
    :global(.btn)
      width: 100%
    p
      margin: 0
  .blocked
    color: var(--c-muted)
  .older
    margin-top: var(--sp-4)
  .added
    position: fixed
    z-index: 11
    bottom: var(--sp-3)
    left: 50%
    transform: translateX(-50%)
    width: max-content
    max-width: calc(100vw - 24px)
    margin: 0
    padding: var(--sp-3)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)
    overflow-wrap: anywhere
  @media (max-width: 600px)
    .history-row
      flex-direction: column
      align-items: flex-start
      gap: var(--sp-1)
      strong
        max-width: 100%
      time
        flex-shrink: 1
        overflow-wrap: anywhere
</style>
