<script lang="ts">
  import { onMount } from "svelte";
  import { request, type Printer, type AmsInventory } from "../lib/api";
  import AmsSlot from "../lib/AmsSlot.svelte";
  const id = window.location.pathname.split("/")[2];
  let printer = $state<Printer>(),
    inventory = $state<AmsInventory>();
  let error = $state(""),
    loading = $state(true),
    busy = $state(false);
  let requested = $state<boolean | null>(null),
    message = $state("");
  const controller = new AbortController();
  let refreshing: Promise<void> | undefined;
  let confirmation: ReturnType<typeof setTimeout> | undefined;
  const refill = $derived(inventory?.auto_refill);
  async function refresh() {
    if (refreshing) return refreshing;
    refreshing = (async () => {
      try {
        const [p, a] = await Promise.all([
          request<Printer>(`/api/printers/${id}`, {
            signal: controller.signal,
          }),
          request<AmsInventory>(`/api/printers/${id}/ams`, {
            signal: controller.signal,
          }),
        ]);
        if (controller.signal.aborted) return;
        printer = p;
        inventory = a;
        if (
          requested !== null &&
          a.current &&
          a.auto_refill.enabled === requested
        ) {
          requested = null;
          message = "設定の反映を確認しました。";
          clearTimeout(confirmation);
        }
      } catch (cause) {
        if (!controller.signal.aborted) {
          error = (cause as Error).message;
          if (inventory)
            inventory = {
              ...inventory,
              current: false,
              slots: inventory.slots.map((s) => ({ ...s, current: false })),
            };
        }
      } finally {
        if (!controller.signal.aborted) loading = false;
      }
    })();
    try {
      await refreshing;
    } finally {
      refreshing = undefined;
    }
  }
  onMount(() => {
    void refresh();
    const timer = setInterval(() => {
      if (!busy && !document.hidden) void refresh();
    }, 5000);
    return () => {
      controller.abort();
      clearInterval(timer);
      clearTimeout(confirmation);
    };
  });
  async function mutate(path: string, data: unknown) {
    if (busy) return false;
    busy = true;
    error = "";
    try {
      await request(path, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(data),
        signal: controller.signal,
      });
      // Complete any earlier poll before fetching the result of this write.
      await refreshing;
      await refresh();
      return true;
    } catch (cause) {
      if (!controller.signal.aborted) {
        error = (cause as Error).message;
        await refresh();
      }
      return false;
    } finally {
      busy = false;
    }
  }
  async function setRefill(enabled: boolean) {
    if (await mutate(`/api/printers/${id}/ams/auto-refill`, { enabled })) {
      if (inventory?.current && refill?.enabled === enabled) {
        message = "設定の反映を確認しました。";
        return;
      }
      requested = enabled;
      message = "設定を送信しました。プリンターの報告を待っています。";
      clearTimeout(confirmation);
      confirmation = setTimeout(() => {
        if (requested !== null) {
          requested = null;
          message =
            "設定の反映を確認できません。状態を更新し、プリンター側の設定を確認してください。";
        }
      }, 20000);
    }
  }
</script>

<svelte:head><title>AMSの材料 · OrcaServer</title></svelte:head>
<section class="content" aria-label="AMSの材料">
  <a href="/printers">プリンター一覧へ</a>
  <div class="page-heading">
    <h1>{printer?.name ?? "プリンター"} · AMS</h1>
    <button
      class="btn"
      disabled={busy}
      onclick={() => {
        error = "";
        void refresh();
      }}>状態を更新</button
    >
  </div>
  {#if error}<div class="notice"><p role="alert">{error}</p></div>{/if}
  {#if loading}<p class="state" role="status">読み込んでいます…</p>
  {:else if inventory}
    {#if !inventory.current}<div class="notice">
        <p role="status">
          現在の装填状態は未確認です。接続後の報告を待ってください。
        </p>
      </div>{/if}
    {#if !inventory.slots.length}<p class="state">
        AMSの情報はまだありません。プリンターとAMSの接続を確認してください。
      </p>{/if}
    <ul class="slots">
      {#each inventory.slots as slot (slot.id)}<AmsSlot
          {slot}
          printerId={id}
          slots={inventory.slots}
          {busy}
          {mutate}
        />{/each}
    </ul>
    <details class="refill">
      <summary>自動補充</summary>
      {#if !inventory.current || refill?.supported == null}<p>
          対応状況は未報告です。
        </p>
      {:else if !refill.supported}<p>プリンターが非対応と報告しています。</p>
      {/if}
      <p aria-label="自動補充の設定状態">
        {!inventory.current || refill?.enabled == null
          ? "設定状態は未報告"
          : refill.enabled
            ? "有効"
            : "無効"}
      </p>
      {#if inventory.current && refill?.supported === true}<button
          class="btn"
          disabled={busy || requested !== null}
          onclick={() => setRefill(refill?.enabled !== true)}
          >{refill.enabled === true ? "無効にする" : "有効にする"}</button
        >{/if}
      {#if message}<p role="status">{message}</p>{/if}
      <p class="help">
        材料切れ後の継続はプリンターのFilament
        Backupが行います。上の使用順は印刷開始時の選択です。補充先の順序はプリンターが決めます。
      </p>
      <p class="help">
        登録上の製品・色が同じでも、機器が同一材料として認識しているかは各スロットの「機器が認識する補充先」で確認してください。未報告の場合はプリンターまたはBambu
        Studioで確認してください。
      </p>
    </details>
  {/if}
  <p>
    <a href="/filaments">材料台帳を開く</a> ·
    <a href={`/printers/${id}`}>プリンター構成を編集</a>
  </p>
</section>

<style lang="sass">
  .page-heading
    margin-top: var(--sp-3)
  .slots
    list-style: none
    padding: 0
    margin: var(--sp-3) 0
  .refill
    margin: var(--sp-4) 0
  p
    margin: var(--sp-2) 0
</style>
