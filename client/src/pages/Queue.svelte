<script lang="ts">
  import { onMount } from "svelte";
  import { request, type Printer, type Filament } from "../lib/api";
  import PrinterQueue from "../lib/PrinterQueue.svelte";
  let printers = $state<Printer[]>([]),
    filaments = $state<Filament[]>([]);
  let loading = $state(true),
    error = $state("");
  let printerId = $state(
    new URLSearchParams(location.search).get("printer_id") ?? "",
  );
  const controller = new AbortController();
  async function load() {
    loading = true;
    error = "";
    try {
      const [p, f] = await Promise.all([
        request<Printer[]>("/api/printers", { signal: controller.signal }),
        request<Filament[]>("/api/filaments", { signal: controller.signal }),
      ]);
      printers = p;
      filaments = f;
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
    } finally {
      loading = false;
    }
  }
  function selectPrinter() {
    const params = new URLSearchParams(location.search);
    if (printerId) params.set("printer_id", printerId);
    else params.delete("printer_id");
    history.replaceState(
      null,
      "",
      `${location.pathname}${params.size ? "?" + params : ""}`,
    );
  }
  onMount(() => {
    void load();
    return () => controller.abort();
  });
</script>

<svelte:head><title>印刷キュー · OrcaServer</title></svelte:head>
<section class="content" aria-label="印刷キュー">
  <h1 class="sr-only">プリンターとキュー</h1>
  {#if loading}<p class="state" role="status">プリンターを読み込んでいます…</p>
  {:else if error}<div class="notice">
      <p role="alert">{error}</p>
      <button class="btn" onclick={() => void load()}>再試行</button>
    </div>
  {:else if printers.length === 0}<p class="state">
      印刷先が登録されていません。<a href="/printers/new">プリンターを追加</a>
    </p>
  {:else}
    {#if printers.length > 1}<label class="field"
        ><span>表示するプリンター</span><select
          bind:value={printerId}
          onchange={selectPrinter}
          ><option value="">すべて</option>{#each printers as p}<option
              value={p.id}>{p.name}</option
            >{/each}</select
        ></label
      >{/if}
    {#if printerId && !printers.some((p) => p.id === printerId)}<p
        class="notice"
      >
        プリンターが見つかりません。<a href="/">一覧へ戻る</a>
      </p>{/if}
    {#each printers.filter((p) => !printerId || p.id === printerId) as printer (printer.id)}<PrinterQueue
        {printer}
        {filaments}
      />{/each}
  {/if}
</section>

<style lang="sass">
  .sr-only
    position: absolute
    width: 1px
    height: 1px
    overflow: hidden
    clip-path: inset(50%)
</style>
