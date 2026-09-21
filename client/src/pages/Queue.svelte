<script lang="ts">
  import { onMount } from "svelte";
  import {
    ApiError,
    request,
    type Printer,
    type Filament,
    type AmsInventory,
  } from "../lib/api";
  import {
    failureText,
    phaseText,
    printerText,
    type Action,
    type Command,
    type QueueState,
  } from "../lib/queue";
  let printers = $state<Printer[]>([]);
  let filaments = $state<Filament[]>([]);
  let inventory = $state<AmsInventory>();
  let printerId = $state(
    new URLSearchParams(window.location.search).get("printer_id") ?? "",
  );
  let printersLoaded = $state(false);
  const queuePath = $derived(
    `/api/queue?printer_id=${encodeURIComponent(printerId)}`,
  );
  let queue = $state<QueueState>();
  let cleared = $state(false);
  let busy = $state(false);
  let pending = $state<Command>();
  let error = $state("");
  let readError = $state("");
  let notice = $state("");
  let reading = $state(false);
  let sequence = 0;
  const controller = new AbortController();
  const choices = $derived({
    next: queue?.allowed.next ? queue.waiting[0] : null,
    retry: queue?.allowed.retry ? queue.current : null,
    discard: queue?.allowed.discard ? queue.current : null,
  });
  const disabled = $derived(busy || !!pending || !!readError || !queue);

  async function loadPrinters() {
    try {
      printers = await request<Printer[]>("/api/printers", {
        signal: controller.signal,
      });
      printersLoaded = true;
      if (!printerId && printers.length === 1) printerId = printers[0].id;
      filaments = await request<Filament[]>("/api/filaments", {
        signal: controller.signal,
      });
      await refresh();
    } catch (cause) {
      if (!controller.signal.aborted) readError = (cause as Error).message;
    }
  }
  function changePrinter() {
    sequence++;
    queue = undefined;
    inventory = undefined;
    cleared = false;
    error = "";
    readError = "";
    notice = "";
    const params = new URLSearchParams(window.location.search);
    params.set("printer_id", printerId);
    history.replaceState(null, "", `/queue?${params}`);
    void refresh();
  }
  function receive(value: QueueState) {
    if (
      queue?.epoch !== value.epoch ||
      queue?.generation !== value.generation ||
      queue?.printer.ready_to_print !== value.printer.ready_to_print
    )
      cleared = false;
    queue = value;
    readError = "";
  }
  async function refresh() {
    if (reading || busy || pending || !printerId) return;
    reading = true;
    const ticket = ++sequence;
    try {
      const value = await request<QueueState>(queuePath, {
        signal: controller.signal,
      });
      const slots = await request<AmsInventory>(
        `/api/printers/${printerId}/ams`,
        { signal: controller.signal },
      );
      if (!controller.signal.aborted && ticket === sequence) {
        receive(value);
        inventory = slots;
      }
    } catch (cause) {
      if (!controller.signal.aborted && ticket === sequence) {
        readError = (cause as Error).message;
        cleared = false;
      }
    } finally {
      reading = false;
    }
  }
  onMount(() => {
    void loadPrinters();
    const timer = setInterval(() => {
      if (!document.hidden) void refresh();
    }, 2000);
    return () => {
      clearInterval(timer);
      controller.abort();
    };
  });
  async function send(action?: Action, basis?: Omit<Command, "action">) {
    if (!queue || busy || (action && pending)) return;
    if (action)
      pending = {
        epoch: basis?.epoch ?? queue.epoch,
        generation: basis?.generation ?? queue.generation,
        request_id: basis?.request_id ?? queue.request_id,
        action,
      };
    if (!pending) return;
    const command = pending;
    busy = true;
    error = "";
    notice = "";
    cleared = false;
    sequence++;
    let rejected = false;
    try {
      const value = await request<QueueState>(queuePath, {
        method: "POST",
        signal: controller.signal,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(command),
      });
      if (controller.signal.aborted) return;
      receive(value);
      pending = undefined;
    } catch (cause) {
      if (controller.signal.aborted) return;
      error = (cause as Error).message;
      if (
        cause instanceof ApiError &&
        cause.status >= 400 &&
        cause.status < 500
      ) {
        pending = undefined;
        rejected = true;
      }
    } finally {
      busy = false;
      if (rejected) {
        void refresh();
      }
    }
  }
</script>

<svelte:head><title>印刷キュー · OrcaServer</title></svelte:head>
<section class="content" aria-label="印刷キュー">
  <div class="page-heading">
    <h1>印刷キュー</h1>
    <a href="/">プレートを選ぶ</a>
  </div>
  <label class="field"
    ><span>プリンター</span><select
      bind:value={printerId}
      onchange={changePrinter}
      disabled={busy || !!pending || reading}
    >
      <option value="" disabled>印刷先を選択</option>
      {#each printers as printer}<option value={printer.id}
          >{printer.name} / {printer.machine_profile_key}</option
        >{/each}
    </select></label
  >
  {#if printersLoaded && printers.length === 0}<p class="state">
      印刷先が登録されていません。<a href="/printers/new">プリンターを追加</a>
    </p>{/if}
  {#if error || readError}
    <div class="notice">
      <p role="alert">{error || readError}</p>
      {#if pending}
        <p>
          送信結果が不明です。別の印刷を始めず、同じ要求の結果を確認します。
        </p>
        <button class="btn" disabled={busy} onclick={() => void send()}
          >同じ要求を再確認</button
        >
      {:else}
        <button
          class="btn"
          disabled={busy}
          onclick={() => {
            error = "";
            void loadPrinters();
          }}>最新状態を読み直す</button
        >
      {/if}
    </div>
  {/if}
  {#if busy}<p class="state" role="status">操作を送信しています…</p>{/if}
  {#if notice}<p class="caption" role="status">{notice}</p>{/if}
  {#if queue}
    <p class="printer-state" aria-live="polite">{printerText(queue.printer)}</p>
    {#if queue.current}
      <section class="current" aria-label="現在の印刷">
        <p class="caption" aria-live="polite">
          {phaseText[queue.current.state]}
        </p>
        <h2 class="job-name">{queue.current.name}</h2>
        <p class="help">
          予定材料: {filaments.find((f) => f.id === queue!.current!.filament_id)
            ?.name ?? "材料を確認"} · {queue.current
            .required_machine_profile_key}
        </p>
        {#if queue.current.actual_ams_slot !== null && queue.current.actual_ams_slot !== undefined}<p
            class="caption"
          >
            使用中: AMS {Math.floor(queue.current.actual_ams_slot / 4) + 1} / スロット
            {(queue.current.actual_ams_slot % 4) + 1}
          </p>{/if}
        {#if queue.current.state === "printing" && queue.printer.synchronized}
          <p>
            {queue.printer.print.percent ??
              "—"}%{#if queue.printer.print.remaining_minutes !== null}
              · 残り約{queue.printer.print.remaining_minutes}分{/if}
          </p>
        {/if}
        {#if queue.current.state === "needs_attention"}
          <div class="notice">
            <p role="alert">
              {failureText[queue.current.last_error ?? ""] ??
                queue.current.last_error ??
                "本体と接続を確認してください。"}
            </p>
            <p>
              印刷が続いている場合は再実行せず、状態の復帰を待ってください。
            </p>
          </div>
        {/if}
      </section>
    {/if}
    {#if queue.waiting[0]}<p class="next">
        次: <strong>{queue.waiting[0].name}</strong>
      </p>{/if}
    {#if choices.next || choices.retry || choices.discard}
      <label class="confirm"
        ><input type="checkbox" bind:checked={cleared} {disabled} /><span
          >造形物を取り外し、空のビルドプレートを戻しました</span
        ></label
      >
      <div class="actions">
        {#if choices.next}<button
            class="btn primary"
            disabled={disabled || !cleared}
            onclick={() =>
              void send({
                type: "next",
                expected_job: choices.next!.id,
                removed_job: queue!.current?.id ?? null,
                cleared: true,
              })}>次を印刷</button
          >{/if}
        {#if choices.retry}<button
            class="btn primary"
            disabled={disabled || !cleared}
            onclick={() =>
              void send({
                type: "retry",
                expected_job: choices.retry!.id,
                cleared: true,
              })}>同じプレートを再印刷</button
          >{/if}
        {#if choices.discard}<button
            class="btn"
            class:primary={!choices.next && !choices.retry}
            disabled={disabled || !cleared}
            onclick={() =>
              void send({
                type: "discard",
                expected_job: choices.discard!.id,
                cleared: true,
              })}
            >{queue.current?.state === "awaiting_removal"
              ? "取り外しを完了"
              : "現在のジョブを除く"}</button
          >{/if}
      </div>
    {/if}
    <h2 class="waiting-heading">待機中（{queue.waiting.length}）</h2>
    {#if queue.waiting.length === 0}<p class="help">
        待機中のプレートはありません。プレートの詳細から追加できます。
      </p>{/if}
    <ol class="plate-list">
      {#each queue.waiting as job, index (job.id)}
        {@const slot = inventory?.slots.find((s) => s.id === job.ams_slot_id)}
        <li class="plate-row" aria-label={job.name}>
          <strong>{index + 1}. {job.name}</strong>
          <span class="caption"
            >予定材料: {filaments.find((f) => f.id === job.filament_id)?.name ??
              "材料を確認"}</span
          >
          <span class="caption"
            >{slot
              ? `AMS ${slot.ams_id} / スロット ${slot.slot_index + 1}`
              : "AMSを確認"} · {job.required_machine_profile_key}</span
          >
          <span class="caption">{job.process_profile_key} · {job.bed_type}</span
          >
          {#if job.hold_reason}<p class="help">
              保留: {failureText[job.hold_reason] ?? job.hold_reason}
            </p>{/if}
          <div class="row-actions">
            <a class="btn" href={`/plates/${job.plate_id}?edit=1`}
              >プレートの条件を編集</a
            >
            <a class="btn" href={`/printers/${printerId}/ams`}>AMSを確認</a>
            <button
              class="btn"
              disabled={disabled || index === 0}
              aria-label={`${job.name}を前へ`}
              onclick={() =>
                void send({ type: "move", job_id: job.id, index: index - 1 })}
              >前へ</button
            ><button
              class="btn"
              disabled={disabled || index === queue!.waiting.length - 1}
              aria-label={`${job.name}を後へ`}
              onclick={() =>
                void send({ type: "move", job_id: job.id, index: index + 1 })}
              >後へ</button
            ><button
              class="btn"
              {disabled}
              aria-label={`${job.name}を削除`}
              onclick={() => void send({ type: "remove", job_id: job.id })}
              >削除</button
            >
          </div>
        </li>
      {/each}
    </ol>
    <p class="help">
      キューは再起動後も残ります。印刷は自動で進まず、毎回空のビルドプレートを確認して開始します。
    </p>
  {:else if !readError}<p class="state" role="status">
      キューを読み込んでいます…
    </p>{/if}
</section>

<style lang="sass">
  .job-name, .next
    overflow-wrap: anywhere
  .help, .printer-state
    font-size: var(--fs-sm)
    color: var(--c-muted)
  .printer-state
    margin: var(--sp-2) 0 var(--sp-4)
  .current
    padding: var(--sp-3)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)
    p:first-child
      margin-top: 0
    p:last-child
      margin-bottom: 0
  .confirm
    display: flex
    align-items: flex-start
    gap: var(--sp-2)
    margin-top: var(--sp-3)
    input
      flex-shrink: 0
      margin-top: var(--sp-1)
  .waiting-heading
    margin-top: var(--sp-5)
  .row-actions
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)
    margin-top: var(--sp-2)
</style>
