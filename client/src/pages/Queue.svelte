<script lang="ts">
  import { onMount } from "svelte";
  import { ApiError, request, type Plate } from "../lib/api";
  import {
    failureText,
    phaseText,
    printerText,
    slotLabel,
    type Action,
    type Command,
    type QueueState,
  } from "../lib/queue";
  let queue = $state<QueueState>();
  let selectedId = $state(
    new URLSearchParams(window.location.search).get("plate"),
  );
  let plate = $state<Plate>();
  let slot = $state(-1);
  let cleared = $state(false);
  let busy = $state(false);
  let pending = $state<Command>();
  let error = $state("");
  let readError = $state("");
  let notice = $state("");
  let reading = false;
  let sequence = 0;
  const controller = new AbortController();
  const ams = $derived(queue?.printer.synchronized ? queue.printer.ams : null);
  const choices = $derived({
    next: queue?.allowed.next ? queue.waiting[0] : null,
    retry: queue?.allowed.retry ? queue.current?.job : null,
    discard: queue?.allowed.discard ? queue.current?.job : null,
  });
  const disabled = $derived(busy || !!pending || !!readError || !queue);

  function receive(value: QueueState) {
    if (
      queue?.generation !== value.generation ||
      queue?.printer.ready_to_print !== value.printer.ready_to_print
    )
      cleared = false;
    queue = value;
    readError = "";
  }
  async function refresh() {
    if (reading || busy || pending) return;
    reading = true;
    const ticket = ++sequence;
    try {
      const value = await request<QueueState>("/api/queue", {
        signal: controller.signal,
      });
      if (!controller.signal.aborted && ticket === sequence) receive(value);
    } catch (cause) {
      if (!controller.signal.aborted && ticket === sequence) {
        readError = (cause as Error).message;
        cleared = false;
      }
    } finally {
      reading = false;
    }
  }
  async function loadPlate() {
    if (!selectedId) return;
    try {
      const saved = await request<Plate>(
        `/api/plates/${encodeURIComponent(selectedId)}`,
        { signal: controller.signal },
      );
      if (!controller.signal.aborted) plate = saved;
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
    }
  }
  onMount(() => {
    void refresh();
    void loadPlate();
    const timer = setInterval(() => {
      if (!document.hidden) void refresh();
    }, 2000);
    return () => {
      clearInterval(timer);
      controller.abort();
    };
  });
  async function send(action?: Action) {
    if (!queue || busy || (action && pending)) return;
    if (action)
      pending = {
        generation: queue.generation,
        request_id: queue.request_id,
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
      const value = await request<QueueState>("/api/queue", {
        method: "POST",
        signal: controller.signal,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(command),
      });
      if (controller.signal.aborted) return;
      receive(value);
      pending = undefined;
      if (command.action.type === "add") {
        selectedId = null;
        plate = undefined;
        slot = -1;
        history.replaceState(null, "", "/queue");
        notice = "キューに追加しました";
      }
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
        if (command.action.type === "add") void loadPlate();
      }
    }
  }
</script>

<svelte:head><title>印刷キュー · OrcaServer</title></svelte:head>
<section class="content" aria-label="印刷キュー">
  <div class="page-heading">
    <h1>{selectedId ? "キューに追加" : "印刷キュー"}</h1>
    <a href={selectedId ? "/queue" : "/"}
      >{selectedId ? "キューへ戻る" : "プレートを選ぶ"}</a
    >
  </div>
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
            void refresh();
            void loadPlate();
          }}>最新状態を読み直す</button
        >
      {/if}
    </div>
  {/if}
  {#if busy}<p class="state" role="status">操作を送信しています…</p>{/if}
  {#if notice}<p class="caption" role="status">{notice}</p>{/if}
  {#if selectedId}
    {#if plate}
      <h2 class="job-name">{plate.name}</h2>
      {#if plate.print}
        <p class="help">追加時のモデル・印刷設定・AMS選択を保存します。</p>
        <label class="field"
          ><span>使用するAMSスロット</span>
          <select bind:value={slot} {disabled}>
            <option value={-1} disabled>スロットを選択</option>
            {#each Array.from({ length: 16 }, (_, i) => i) as i}<option
                value={i}>{slotLabel(i, ams)}</option
              >{/each}
          </select>
        </label>
        <p class="help">
          キューはサーバー再起動で消えます。保存済みプレートは残ります。
        </p>
        <div class="actions">
          <button
            class="btn primary"
            disabled={disabled || slot < 0}
            onclick={() =>
              void send({
                type: "add",
                plate_id: plate!.id,
                revision: plate!.revision,
                ams_slot: slot,
              })}>キューに追加</button
          ><a class="btn" href={`/plates/${plate.id}`}>プレートへ戻る</a>
        </div>
      {:else}<p>先にプレートを配置・スライスしてください。</p>
        <a href={`/plates/${plate.id}`}>プレートへ戻る</a>{/if}
    {:else if !error}<p class="state" role="status">
        プレートを読み込んでいます…
      </p>{/if}
  {:else if queue}
    <p class="printer-state" aria-live="polite">{printerText(queue.printer)}</p>
    {#if queue.current}
      <section class="current" aria-label="現在の印刷">
        <p class="caption" aria-live="polite">
          {phaseText[queue.current.phase]}
        </p>
        <h2 class="job-name">{queue.current.job.name}</h2>
        <p class="help">
          {slotLabel(queue.current.job.ams_slot, ams)}
        </p>
        {#if queue.current.phase === "printing" && queue.printer.synchronized}
          <p>
            {queue.printer.print.percent ??
              "—"}%{#if queue.printer.print.remaining_minutes !== null}
              · 残り約{queue.printer.print.remaining_minutes}分{/if}
          </p>
        {/if}
        {#if queue.current.phase === "needs_attention"}
          <div class="notice">
            <p role="alert">
              {failureText[queue.current.message ?? ""] ??
                queue.current.message ??
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
            >{queue.current?.phase === "awaiting_removal"
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
        <li class="plate-row" aria-label={job.name}>
          <strong>{index + 1}. {job.name}</strong><span class="caption"
            >{slotLabel(job.ams_slot, ams)}</span
          >
          <div class="row-actions">
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
      キューはサーバー再起動で消えます。保存済みプレートは残ります。
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
