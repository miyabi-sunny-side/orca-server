<script lang="ts">
  import { onMount } from "svelte";
  import {
    ApiError,
    request,
    type Plate,
    type Printer,
    type Filament,
  } from "../lib/api";
  import StlPreview from "../lib/StlPreview.svelte";
  import PlateEditor from "../lib/PlateEditor.svelte";
  import { choosePrinter, destinations, emptyConditions } from "../lib/plate";
  import { failureText, type Command, type QueueState } from "../lib/queue";
  let { id }: { id: string } = $props();
  let plate = $state<Plate>(),
    loading = $state(true),
    error = $state("");
  let editing = $state(new URLSearchParams(location.search).has("edit"));
  let printers = $state<Printer[]>([]),
    filaments = $state<Filament[]>([]),
    printerId = $state("");
  let queue = $state<QueueState>(),
    readError = $state(""),
    reading = $state(false),
    busy = $state(false),
    notice = $state("");
  let pending = $state<{ printerId: string; command: Command }>();
  let selectedModelId = $state("");
  const selectedModel = $derived(
    plate?.models.find((model) => model.id === selectedModelId) ??
      plate?.models[0],
  );
  const conditions = $derived(plate?.conditions ?? emptyConditions);
  const candidates = $derived(
    destinations(printers, conditions.required_machine_profile_key),
  );
  const hold = $derived(
    !conditions.required_machine_profile_key
      ? "プレートの印刷条件を設定してください。"
      : !candidates.length
        ? "要求する機種・ノズルに一致するプリンターがありません。"
        : queue?.admission?.reason
          ? (failureText[queue.admission.reason] ?? queue.admission.reason)
          : "",
  );
  const disabled = $derived(
    busy ||
      !!pending ||
      reading ||
      !!readError ||
      !queue?.admission?.allowed ||
      queue.admission.plate_version !== plate?.version,
  );
  const pendingKey = $derived(`orca-plate-pending:${id}`),
    printerKey = $derived(`orca-plate-printer:${id}`);
  const controller = new AbortController();
  let sequence = 0;
  async function load() {
    loading = true;
    error = "";
    try {
      const saved = await request<Plate>(`/api/plates/${id}`, {
        signal: controller.signal,
      });
      if (!controller.signal.aborted) {
        plate = saved;
        await refresh();
      }
    } catch (e) {
      if (!controller.signal.aborted) error = (e as Error).message;
    } finally {
      if (!controller.signal.aborted) loading = false;
    }
  }
  async function refresh() {
    if (reading || busy || !plate || editing) return;
    reading = true;
    const ticket = ++sequence;
    try {
      const [p, f] = await Promise.all([
        request<Printer[]>("/api/printers", { signal: controller.signal }),
        request<Filament[]>("/api/filaments", { signal: controller.signal }),
      ]);
      if (controller.signal.aborted || ticket !== sequence) return;
      printers = p;
      filaments = f;
      printerId =
        pending?.printerId ??
        choosePrinter(
          destinations(p, conditions.required_machine_profile_key),
          printerId,
        );
      const value = printerId
        ? await request<QueueState>(
            `/api/queue?printer_id=${encodeURIComponent(printerId)}&plate_id=${id}`,
            { signal: controller.signal },
          )
        : undefined;
      if (controller.signal.aborted || ticket !== sequence) return;
      queue = value;
      readError = "";
      if (
        value?.admission &&
        value.admission.plate_version !== plate.version &&
        !pending
      ) {
        plate = await request<Plate>(`/api/plates/${id}`, {
          signal: controller.signal,
        });
        queue = undefined;
      }
    } catch (e) {
      if (!controller.signal.aborted && ticket === sequence) {
        queue = undefined;
        readError = (e as Error).message;
      }
    } finally {
      if (!controller.signal.aborted) reading = false;
    }
  }
  function rememberPrinter() {
    queue = undefined;
    notice = "";
    try {
      localStorage.setItem(printerKey, printerId);
    } catch {
      /* Selection remains usable for this page. */
    }
    void refresh();
  }
  function savePending() {
    try {
      if (pending) sessionStorage.setItem(pendingKey, JSON.stringify(pending));
      else sessionStorage.removeItem(pendingKey);
    } catch {
      /* The in-memory request still fences retries. */
    }
  }
  async function add() {
    if (busy || (!pending && disabled) || !plate) return;
    if (!pending && queue)
      pending = {
        printerId,
        command: {
          epoch: queue.epoch,
          generation: queue.generation,
          request_id: queue.request_id,
          action: {
            type: "add",
            plate_id: plate.id,
            plate_version: plate.version,
          },
        },
      };
    if (!pending) return;
    savePending();
    busy = true;
    error = "";
    notice = "";
    sequence++;
    try {
      await request<QueueState>(
        `/api/queue?printer_id=${encodeURIComponent(pending.printerId)}`,
        {
          method: "POST",
          signal: controller.signal,
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(pending.command),
        },
      );
      if (controller.signal.aborted) return;
      pending = undefined;
      savePending();
      queue = undefined;
      notice = "キューに追加しました";
    } catch (e) {
      if (controller.signal.aborted) return;
      error = (e as Error).message;
      if (e instanceof ApiError && e.status >= 400 && e.status < 500) {
        pending = undefined;
        savePending();
        queue = undefined;
      }
    } finally {
      if (!controller.signal.aborted) {
        busy = false;
        void refresh();
      }
    }
  }
  onMount(() => {
    try {
      printerId = localStorage.getItem(printerKey) ?? "";
      const saved = JSON.parse(sessionStorage.getItem(pendingKey) ?? "null");
      if (
        saved?.command?.action?.type === "add" &&
        saved.command.action.plate_id === id &&
        typeof saved.printerId === "string"
      )
        pending = saved;
    } catch {
      /* Ignore invalid browser storage. */
    }
    void load();
    const timer = setInterval(() => {
      if (!document.hidden) void refresh();
    }, 5000);
    return () => {
      clearInterval(timer);
      controller.abort();
    };
  });
</script>

<svelte:head
  ><title>{plate?.name ?? "プレート"} · OrcaServer</title></svelte:head
>
<section class="content" aria-label="プレート詳細">
  <a href="/">プレート一覧へ</a>
  {#if loading}<p class="state" role="status">プレートを読み込んでいます…</p>
  {:else if plate}
    <div class="detail-layout" class:editing>
      <div class="controls">
        <div class="page-heading"><h1>{plate.name}</h1></div>
        {#if editing && !pending}
          <PlateEditor
            initial={plate}
            saved={(value) => {
              plate = value;
              editing = false;
              history.replaceState(null, "", `/plates/${id}`);
              void refresh();
            }}
            cancel={() => {
              editing = false;
              history.replaceState(null, "", `/plates/${id}`);
              void load();
            }}
          />
        {:else}
          <div class="conditions" aria-label="保存した印刷条件">
            <p>
              {conditions.required_machine_profile_key ?? "機種・ノズル未設定"}
            </p>
            <p>
              {filaments.find((f) => f.id === conditions.filament_id)?.name ??
                (conditions.filament_id
                  ? "材料を確認中"
                  : "フィラメント未設定")}
            </p>
            <p>
              {conditions.process_profile_key ?? "工程未設定"} · {conditions.bed_type ??
                "ビルドプレート未設定"}
            </p>
          </div>
          {#if candidates.length > 1}<label class="field"
              ><span>追加先のプリンター</span><select
                bind:value={printerId}
                onchange={rememberPrinter}
                disabled={busy || !!pending || reading}
                >{#each candidates as printer}<option value={printer.id}
                    >{printer.name}</option
                  >{/each}</select
              ></label
            >
          {:else if candidates.length === 1}<p class="caption">
              追加先: {candidates[0].name}
            </p>{/if}
          <div class="actions">
            <button class="btn primary" {disabled} onclick={() => void add()}
              >印刷キューへ</button
            >
            <button
              class="btn"
              disabled={busy || !!pending}
              onclick={() => {
                editing = true;
                notice = "";
                sequence++;
              }}>構成を編集</button
            >
          </div>
          {#if busy}<p role="status">キューに追加しています…</p>{/if}
          {#if notice}<p role="status">
              {notice}。<a
                href={`/queue?printer_id=${encodeURIComponent(printerId)}`}
                >キューを見る</a
              >
            </p>{/if}
          {#if pending && !busy}<div class="notice">
              <p>
                追加の結果が不明です。キューを確認し、同じ要求の結果を再確認してください。
              </p>
              <div class="actions">
                <button class="btn" onclick={() => void add()}
                  >同じ要求を再確認</button
                ><a
                  href={`/queue?printer_id=${encodeURIComponent(pending.printerId)}`}
                  >キューを見る</a
                >
              </div>
            </div>{/if}
          {#if hold && !pending}<div class="notice">
              <p>{hold}</p>
              <div class="actions">
                {#if printerId}<a href={`/printers/${printerId}/ams`}
                    >AMSを確認</a
                  >{:else}<a href="/printers">プリンターを確認</a>{/if}
              </div>
            </div>{/if}
          {#if readError}<div class="notice">
              <p role="alert">{readError}</p>
              <button class="btn" onclick={() => void refresh()}
                >追加条件を読み直す</button
              >
            </div>{/if}
          <ul class="plate-list">
            {#each plate.models as model}<li class="plate-row">
                <button
                  class="model-choice"
                  aria-pressed={selectedModel?.id === model.id}
                  onclick={() => (selectedModelId = model.id)}
                >
                  <strong>{model.name}</strong><span>{model.quantity}個</span>
                  <span class="caption"
                    >{selectedModel?.id === model.id
                      ? "表示中"
                      : "形状を見る"}</span
                  >
                </button>
                {#if model.source}<span class="caption"
                    >SCAD参照: {model.source}</span
                  >{:else}<a
                    href={`/api/plates/${plate.id}/files/${model.id}`}
                    download={model.name}>アップロードした元STLを取得</a
                  >{/if}
              </li>{/each}
          </ul>
          <p class="caption">
            SCADモデルは準備開始時に最新データを取得します。条件の編集は未準備の待機分へ反映されます。印刷はキューで手動開始します。
          </p>
        {/if}
      </div>
      {#if !editing && selectedModel}<StlPreview
          name={selectedModel.name}
          url={`/api/plates/${encodeURIComponent(plate.id)}/models/${encodeURIComponent(selectedModel.id)}`}
        />{/if}
    </div>
  {/if}
  {#if error}<div class="notice">
      <p role="alert">{error}</p>
      {#if !pending}<button class="btn" onclick={() => void load()}
          >読み直す</button
        >{/if}
    </div>{/if}
</section>

<style lang="sass">
  section.content
    max-width: 1200px
  .detail-layout
    display: grid
    gap: var(--sp-5)
    grid-template-columns: minmax(0, 1fr)
    .controls
      min-width: 0
    &.editing
      max-width: 720px
      margin: auto
  .model-choice
    display: flex
    flex-direction: column
    align-items: start
    gap: var(--sp-1)
    width: 100%
    font: inherit
    text-align: left
    color: inherit
    background: transparent
    border: 0
    border-radius: var(--radius-sm)
    padding: var(--sp-2)
    cursor: pointer
    overflow-wrap: anywhere
    &[aria-pressed="true"], &:hover
      background: var(--c-hover-1)
  @media (min-width: 768px)
    .detail-layout:not(.editing)
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr)

  .conditions
    margin-bottom: var(--sp-3)
    overflow-wrap: anywhere
    p
      margin: var(--sp-1) 0
    p:not(:first-child)
      color: var(--c-muted)
      font-size: var(--fs-sm)
</style>
