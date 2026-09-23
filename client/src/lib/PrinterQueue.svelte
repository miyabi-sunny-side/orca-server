<script lang="ts">
  import { onMount, tick } from "svelte";
  import {
    ApiError,
    request,
    type Printer as Device,
    type Filament,
    type AmsInventory,
  } from "../lib/api";
  import {
    estimateText,
    failureMessage,
    jobStatus,
    moveIndex,
    type Job,
    printerText,
    type Action,
    type Command,
    type QueueState,
  } from "../lib/queue";
  import Icon from "./Icon.svelte";
  let { printer, filaments }: { printer: Device; filaments: Filament[] } =
    $props();
  const printerId = $derived(printer.id);
  let inventory = $state<AmsInventory>();
  const queuePath = $derived(
    `/api/queue?printer_id=${encodeURIComponent(printerId)}`,
  );
  let queue = $state<QueueState>();
  let busy = $state(false);
  let pending = $state<Command>();
  let error = $state("");
  let readError = $state("");
  let notice = $state("");
  let reading = $state(false);
  let sequence = 0;
  let expanded = $state<Record<string, boolean>>({
    [location.hash.replace(/^#job-/, "")]: true,
  });
  const controller = new AbortController();
  const disabled = $derived(busy || !!pending || !!readError || !queue);

  function receive(value: QueueState) {
    if (
      drag &&
      (drag.basis.epoch !== value.epoch ||
        drag.basis.generation !== value.generation ||
        !value.waiting.some((j) => j.id === drag!.id))
    ) {
      drag = undefined;
      notice = "キューが更新されました。順序を確認してください。";
    }
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
      }
    } finally {
      reading = false;
    }
  }
  onMount(() => {
    void refresh();
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

  type Drag = {
    id: string;
    mode: "pointer" | "keyboard";
    active: boolean;
    x: number;
    y: number;
    target: string;
    after: boolean;
    ids: string[];
    basis: Omit<Command, "action">;
  };
  let drag = $state<Drag>();
  let list = $state<HTMLOListElement>();
  function startDrag(job: Job, mode: Drag["mode"], x = 0, y = 0) {
    if (disabled || !queue) return;
    drag = {
      id: job.id,
      mode,
      active: mode === "keyboard",
      x,
      y,
      target: job.id,
      after: false,
      ids: queue.waiting.map((j) => j.id),
      basis: {
        epoch: queue.epoch,
        generation: queue.generation,
        request_id: queue.request_id,
      },
    };
    if (mode === "keyboard")
      notice = "上下キーで移動し、Enterで確定、Escapeで取消します。";
  }
  function pointerDown(event: PointerEvent, job: Job) {
    if (event.button !== 0) return;
    startDrag(job, "pointer", event.clientX, event.clientY);
    if (drag)
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }
  function pointerMove(event: PointerEvent) {
    if (!drag || drag.mode !== "pointer") return;
    if (
      Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 6 &&
      !drag.active
    )
      return;
    drag.active = true;
    const rows = [
      ...(list?.querySelectorAll<HTMLElement>(".waiting-job") ?? []),
    ].filter((row) => row.dataset.jobId !== drag!.id);
    const target =
      rows.find((row) => event.clientY < row.getBoundingClientRect().bottom) ??
      rows.at(-1);
    if (target) {
      drag.target = target.dataset.jobId!;
      const rect = target.getBoundingClientRect();
      drag.after = event.clientY > rect.top + rect.height / 2;
    }
    if (event.clientY > innerHeight - 40) window.scrollBy(0, 18);
    else if (event.clientY < 60) window.scrollBy(0, -18);
  }
  async function finishDrag() {
    const done = drag;
    drag = undefined;
    if (!done?.active) return;
    const index = moveIndex(done.ids, done.id, done.target, done.after);
    if (index !== null) {
      await send({ type: "move", job_id: done.id, index }, done.basis);
      if (!error) notice = `${index + 1}番目へ移動しました`;
    }
    await tick();
    document.getElementById(`handle-${done.id}`)?.focus();
  }
  function handleKey(event: KeyboardEvent, job: Job) {
    if (["Enter", " "].includes(event.key)) {
      event.preventDefault();
      if (drag?.mode === "keyboard" && drag.id === job.id) void finishDrag();
      else startDrag(job, "keyboard");
    } else if (event.key === "Escape" && drag) {
      event.preventDefault();
      drag = undefined;
      notice = "並べ替えを取消しました";
    } else if (
      drag?.mode === "keyboard" &&
      drag.id === job.id &&
      ["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)
    ) {
      event.preventDefault();
      const old =
        moveIndex(drag.ids, drag.id, drag.target, drag.after) ??
        drag.ids.indexOf(drag.id);
      const index = Math.max(
        0,
        Math.min(
          drag.ids.length - 1,
          event.key === "Home"
            ? 0
            : event.key === "End"
              ? drag.ids.length - 1
              : old + (event.key === "ArrowUp" ? -1 : 1),
        ),
      );
      const others = drag.ids.filter((id) => id !== drag!.id);
      drag.target = others[index] ?? others[others.length - 1];
      drag.after = index >= others.length;
      notice = `${index + 1}番目へ移動。Enterで確定します。`;
    }
  }
</script>

{#snippet details(job: Job)}
  {@const slot = inventory?.slots.find((s) => s.id === job.ams_slot_id)}
  {@const material =
    filaments.find((f) => f.id === job.filament_id)?.name ?? "材料未設定"}
  {@const reason = job.estimate?.error || job.hold_reason || job.last_error}
  <div class="job-details">
    <p class="full-name">{job.name}</p>
    <p>予定材料: {material}</p>
    <p>
      {slot
        ? `AMS ${slot.ams_id} / スロット ${slot.slot_index + 1}`
        : "AMSを確認"} · {job.required_machine_profile_key}
    </p>
    <p>{job.process_profile_key} · {job.bed_type}</p>
    {#if job.actual_ams_slot !== null && job.actual_ams_slot !== undefined}<p>
        使用中: AMS {Math.floor(job.actual_ams_slot / 4) + 1} / スロット {(job.actual_ams_slot %
          4) +
          1}
      </p>{/if}
    {#if job.state !== "queued"}<p>
        推定所要時間: {estimateText(job.estimate)}
      </p>{/if}
    {#if reason}<p class="failure" role="alert">
        {failureMessage(reason, job, material)}
      </p>{/if}
    {#if job.plate_deleted}<p>
        一覧から削除済み · このジョブは継続できます
      </p>{/if}
    <div class="row-actions">
      {#if job.filament_id && (reason === "Selected build plate temperature is missing or zero for this material" || reason === "Configure this material for the required machine and nozzle first" || reason?.includes("Build plate does not support the selected material"))}<a
          class="btn"
          href={`/filaments/${job.filament_id}?machine=${encodeURIComponent(job.required_machine_profile_key ?? "")}&return_queue=${encodeURIComponent(printerId)}&return_job=${job.id}`}
          >材料の温度を設定</a
        >{/if}
      {#if !job.plate_deleted}<a
          class="btn"
          href={`/plates/${job.plate_id}?edit=1`}>プレートの条件を編集</a
        >{/if}
      <a class="btn" href={`/printers/${printerId}/ams`}>AMSを確認</a>
      {#if job.state === "queued"}
        {#if job.estimate?.state === "failed"}<button
            class="btn"
            {disabled}
            onclick={() => void send({ type: "reestimate", job_id: job.id })}
            >再試算</button
          >{/if}
        <button
          class="btn"
          {disabled}
          aria-label={`${job.name}をキューから削除`}
          onclick={() => void send({ type: "remove", job_id: job.id })}
          >キューから削除</button
        >
      {/if}
    </div>
  </div>
{/snippet}

<section class="printer-queue" aria-label={`${printer.name}のキュー`}>
  <div class="printer-heading">
    <h2>{printer.name}</h2>
    <a href={`/printers/${printerId}`}>設定</a>
  </div>
  {#if error || readError}<div class="notice">
      <p role="alert">{error || readError}</p>
      {#if pending}<p>
          送信結果が不明です。別の印刷を始めず、同じ要求の結果を確認します。
        </p>
        <button class="btn" disabled={busy} onclick={() => void send()}
          >同じ要求を再確認</button
        >
      {:else}<button class="btn" disabled={busy} onclick={() => void refresh()}
          >最新状態を読み直す</button
        >{/if}
    </div>{/if}
  <p class="sr-only" role="status">{notice}</p>
  {#if queue}
    {#if queue.printer.connection !== "connected" || !queue.printer.synchronized || queue.printer.print.error}<p
        class="printer-state"
      >
        {printerText(queue.printer)}
      </p>{/if}
    {#if queue.current}
      {#key queue.current.id}<details
          class="job current-job"
          aria-label="現在の印刷"
          id={`job-${queue.current.id}`}
          bind:open={expanded[queue.current.id]}
        >
          <summary class="job-summary"
            ><span class="job-lines"
              ><strong>{queue.current.name}</strong><span class="caption"
                >{jobStatus(queue.current, queue.printer)}</span
              ></span
            ><span class="disclosure"><Icon name="chevron-left" /></span
            ></summary
          >
          {@render details(queue.current)}
        </details>{/key}
    {/if}
    {#if queue.current?.state === "needs_attention"}
      <div class="actions">
        <button
          class="btn primary"
          disabled={disabled || !queue.allowed.retry}
          onclick={() =>
            void send({
              type: "retry",
              expected_job: queue!.current!.id,
              cleared: true,
            })}>取り外した・最初から再印刷</button
        >
        <button
          class="btn"
          disabled={disabled || !queue.allowed.discard}
          onclick={() =>
            void send({
              type: "discard",
              expected_job: queue!.current!.id,
              cleared: true,
            })}>取り外した・現在のジョブを除く</button
        >
      </div>
      {#if queue.recovery?.retry_reason}
        <p class="failure" role="status">
          {failureMessage(queue.recovery.retry_reason)}
        </p>
      {/if}
      {#if queue.recovery?.discard_reason && queue.recovery.discard_reason !== queue.recovery.retry_reason}
        <p class="failure" role="status">
          {failureMessage(queue.recovery.discard_reason)}
        </p>
      {/if}
    {/if}
    {#if !queue.current || queue.current.state === "awaiting_removal"}
      {#if queue.waiting[0]}<div class="actions">
          <button
            class="btn primary"
            disabled={disabled || !queue.allowed.next}
            onclick={() =>
              void send({
                type: "next",
                expected_job: queue!.waiting[0].id,
                removed_job: queue!.current?.id ?? null,
                cleared: true,
              })}
            >{queue.current
              ? "取り外した・次を印刷"
              : "空のプレートで印刷を開始"}</button
          >
        </div>
      {:else if queue.current}<div class="actions">
          <button
            class="btn primary"
            disabled={disabled || !queue.allowed.discard}
            onclick={() =>
              void send({
                type: "discard",
                expected_job: queue!.current!.id,
                cleared: true,
              })}>取り外した</button
          >
        </div>{/if}
    {/if}
    <h3 class="waiting-heading">待機中（{queue.waiting.length}）</h3>
    {#if !queue.waiting.length}<p class="caption">
        待機中のプレートはありません。<a href="/plates">プレートを選ぶ</a>
      </p>{/if}
    <ol class="waiting-list" bind:this={list}>
      {#each queue.waiting as job (job.id)}
        <li
          class="job waiting-job"
          data-job-id={job.id}
          class:dragging={drag?.active && drag.id === job.id}
          class:insert-before={drag?.active &&
            drag.target === job.id &&
            !drag.after}
          class:insert-after={drag?.active &&
            drag.target === job.id &&
            drag.after}
        >
          <button
            class="drag-handle"
            id={`handle-${job.id}`}
            aria-label={`${job.name}を並べ替え`}
            aria-pressed={drag?.active && drag.id === job.id}
            {disabled}
            onpointerdown={(e) => pointerDown(e, job)}
            onpointermove={pointerMove}
            onpointerup={() => void finishDrag()}
            onpointercancel={() => {
              drag = undefined;
            }}
            onkeydown={(e) => handleKey(e, job)}><Icon name="menu" /></button
          >
          <details id={`job-${job.id}`} bind:open={expanded[job.id]}>
            <summary class="job-summary"
              ><span class="job-lines"
                ><strong>{job.name}</strong><span class="caption"
                  >{jobStatus(job)}</span
                ></span
              ><span class="disclosure"><Icon name="chevron-left" /></span
              ></summary
            >
            {@render details(job)}
          </details>
        </li>
      {/each}
    </ol>
  {:else if !readError}<p class="state" role="status">
      キューを読み込んでいます…
    </p>{/if}
</section>

<style lang="sass">
  .printer-queue
    margin-bottom: var(--sp-5)
  .printer-heading
    display: flex
    align-items: baseline
    justify-content: space-between
    gap: var(--sp-3)
    margin-bottom: var(--sp-3)
    h2
      margin: 0
      font-size: var(--fs-xl)
      overflow-wrap: anywhere
    a
      flex-shrink: 0
      font-size: var(--fs-sm)
  .printer-state
    color: var(--c-muted)
    font-size: var(--fs-sm)
  .waiting-heading
    margin: var(--sp-3) 0 var(--sp-2)
    font-size: var(--fs-sm)
    font-weight: 500
    color: var(--c-muted)
  .waiting-list
    list-style: none
    padding: 0
    margin: 0
    display: grid
    gap: var(--sp-2)
  .job
    position: relative
    min-width: 0
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)
  .job-summary
    display: flex
    align-items: center
    gap: var(--sp-2)
    padding: 8px 10px
    min-height: 60px
    cursor: pointer
    list-style: none
    &::-webkit-details-marker
      display: none
    &:hover
      background: var(--c-hover-1)
  .waiting-job .job-summary
    padding-left: 48px
  .job-lines
    display: flex
    flex-direction: column
    min-width: 0
    flex: 1
    strong, .caption
      display: block
      overflow: hidden
      text-overflow: ellipsis
      white-space: nowrap
    strong
      font-size: var(--fs-lg)
      line-height: 1.4
  .disclosure
    color: var(--c-muted)
    transform: rotate(-90deg)
    flex-shrink: 0
  details[open] > summary .disclosure
    transform: rotate(90deg)
  .drag-handle
    position: absolute
    left: 0
    top: 8px
    width: 44px
    height: 44px
    display: grid
    place-items: center
    border: 0
    background: transparent
    color: var(--c-muted)
    cursor: grab
    touch-action: none
    z-index: 1
  .dragging
    outline: 2px solid var(--c-accent)
  .insert-before::before, .insert-after::after
    content: ""
    position: absolute
    left: 0
    right: 0
    height: 3px
    background: var(--c-accent)
    pointer-events: none
  .insert-before::before
    top: -6px
  .insert-after::after
    bottom: -6px
  .job-details
    border-top: 1px solid var(--c-border)
    padding: var(--sp-3)
    font-size: var(--fs-sm)
    overflow-wrap: anywhere
    p
      margin: 0 0 var(--sp-2)
  .full-name
    font-weight: 600
    color: var(--c-on-surface)
  .failure
    color: var(--c-danger)
  .actions .btn
    min-height: 44px
    white-space: normal
  .row-actions
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)
  .sr-only
    position: absolute
    width: 1px
    height: 1px
    overflow: hidden
    clip-path: inset(50%)
</style>
