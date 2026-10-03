<script lang="ts">
  import { contextMenu } from "./context-menu";
  import { onMount, tick } from "svelte";
  import {
    ApiError,
    controlText,
    request,
    sendControl,
    type Printer as Device,
    type Filament,
    type AmsInventory,
  } from "../lib/api";
  import {
    estimateText,
    failureMessage,
    errorLine,
    failureLines,
    queueFailure,
    jobStatus,
    moveIndex,
    menuReasons,
    feedLabel,
    jobControls,
    liveLine,
    type Job,
    printerText,
    type Action,
    type Command,
    type QueueState,
  } from "../lib/queue";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";
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
  let controlling = $state(false),
    stopArmed = $state(false),
    controlNotice = $state("");
  /** Pause, resume or stop the print on the printer itself; the queue follows its reports. */
  async function printControl(action: "pause" | "resume" | "stop") {
    if (action === "stop" && !stopArmed) {
      stopArmed = true;
      setTimeout(() => (stopArmed = false), 4000);
      return;
    }
    stopArmed = false;
    controlling = true;
    try {
      controlNotice = controlText(await sendControl(printerId, { action }));
    } catch (e) {
      controlNotice = (e as Error).message;
    } finally {
      controlling = false;
      void refresh();
    }
  }
  let menu = $state<{
    id: string;
    plate_id: string;
    name: string;
    printerId: string;
    index: number;
  }>();
  let menuRow: HTMLElement | undefined;
  let settingsLink = $state<HTMLAnchorElement>();
  const menuJob = $derived(
    menu?.printerId === printerId
      ? [queue?.current, ...(queue?.waiting ?? [])].find(
          (j) => j?.id === menu?.id,
        )
      : undefined,
  );
  const reasons = $derived(menuReasons(menuJob ?? undefined, queue?.admission));
  function showMenu(job: Job, row: HTMLElement) {
    drag = undefined;
    menuRow = row;
    menu = {
      id: job.id,
      plate_id: job.plate_id,
      name: job.name,
      printerId,
      index: queue?.waiting.findIndex((j) => j.id === job.id) ?? 0,
    };
    if (queue) queue.admission = null;
    void refresh();
  }
  function openMenu(event: Event, job: Job) {
    event.preventDefault();
    showMenu(job, event.currentTarget as HTMLElement);
  }
  async function closeMenu() {
    const index = menu?.index ?? 0;
    menu = undefined;
    await tick();
    const rows = list?.querySelectorAll<HTMLElement>("summary");
    (menuRow?.isConnected
      ? menuRow
      : (rows?.[Math.min(Math.max(index, 0), rows.length - 1)] ?? settingsLink)
    )?.focus();
  }
  function menuKey(event: KeyboardEvent, job: Job) {
    if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"))
      openMenu(event, job);
  }
  async function duplicate() {
    if (disabled || reasons.duplicate || !menuJob || !queue?.admission) return;
    await send({
      type: "add",
      plate_id: menuJob.plate_id,
      plate_version: queue.admission.plate_version,
    });
  }

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
    const target = menu?.id;
    const path =
      queuePath +
      (menu ? `&plate_id=${encodeURIComponent(menu.plate_id)}` : "");
    try {
      const value = await request<QueueState>(path, {
        signal: controller.signal,
      });
      const slots = await request<AmsInventory>(
        `/api/printers/${printerId}/ams`,
        { signal: controller.signal },
      );
      if (
        !controller.signal.aborted &&
        ticket === sequence &&
        target === menu?.id
      ) {
        receive(value);
        inventory = slots;
      }
    } catch (cause) {
      if (
        !controller.signal.aborted &&
        ticket === sequence &&
        target === menu?.id
      ) {
        readError = (cause as Error).message;
      }
    } finally {
      reading = false;
      if (!controller.signal.aborted && target !== menu?.id) void refresh();
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
    let applied = false;
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
      applied = true;
    } catch (cause) {
      if (controller.signal.aborted) return;
      error = failureMessage((cause as Error).message);
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
    const { action: done } = command;
    if (applied && (done.type === "add" || done.type === "remove")) {
      notice =
        done.type === "add" ? "キューを複製しました" : "キューから削除しました";
      if (menu) await closeMenu();
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
    menuKey(event, job);
    if (event.defaultPrevented) return;
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

{#snippet issue()}
  {#if error || readError}<div class="notice">
      <p role="alert">{error || readError}</p>
      {#if pending}<button
          class="btn"
          disabled={busy}
          onclick={() => void send()}>結果を再確認</button
        >
      {:else}<button
          class="icon-btn large"
          disabled={busy}
          aria-label="読み直す"
          title="読み直す"
          onclick={() => void refresh()}><Icon name="refresh-cw" /></button
        >{/if}
    </div>{/if}
{/snippet}

{#snippet summary(job: Job)}
  <summary
    class="job-summary"
    aria-haspopup="dialog"
    use:contextMenu={(row) => showMenu(job, row)}
  >
    <span class="job-lines"
      ><strong>{job.name}</strong><span class="caption"
        >{jobStatus(job, queue?.printer)}</span
      ></span
    >
    <span class="disclosure"><Icon name="chevron-left" /></span>
  </summary>
{/snippet}

{#snippet details(job: Job)}
  {@const slot = inventory?.slots.find((s) => s.id === job.ams_slot_id)}
  {@const material =
    filaments.find((f) => f.id === job.filament_id)?.name ?? "材料未設定"}
  {@const reason = job.estimate?.error || job.hold_reason || job.last_error}
  <div class="job-details">
    <p class="full-name">{job.name}</p>
    <p>予定材料: {material}</p>
    <p>
      {job.feed === "external"
        ? feedLabel(job.feed)
        : slot
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
    {#if job.failure}{#each failureLines(job.failure).lines as line (line)}<p
          class="caption"
        >
          {line}
        </p>{/each}{/if}
    {#if job.plate_deleted}<p>
        一覧から削除済み · このジョブは継続できます
      </p>{/if}
    <div class="row-actions">
      {#if job.filament_id && (reason === "Selected build plate temperature is missing or zero for this material" || reason === "Configure this material for the required machine and nozzle first" || reason?.includes("Build plate does not support the selected material"))}<a
          class="btn"
          href={`/filaments/${job.filament_id}?machine=${encodeURIComponent(job.required_machine_profile_key ?? "")}&return_queue=${encodeURIComponent(printerId)}&return_job=${job.id}`}
          >温度を設定</a
        >{/if}
      {#if !job.plate_deleted}<a
          class="icon-btn large"
          href={`/plates/${job.plate_id}?edit=1`}
          aria-label="プレートの条件を編集"
          title="プレートの条件を編集"><Icon name="pencil" /></a
        >{/if}
      {#if job.feed !== "external"}<a
          class="btn"
          href={`/printers/${printerId}/ams`}>AMS</a
        >{/if}
      {#if job.state === "queued"}
        {#if job.estimate?.state === "failed"}<button
            class="btn"
            {disabled}
            onclick={() => void send({ type: "reestimate", job_id: job.id })}
            >再試算</button
          >{/if}
        <button
          class="icon-btn large"
          {disabled}
          aria-label={`${job.name}をキューから削除`}
          title="キューから削除"
          onclick={() => void send({ type: "remove", job_id: job.id })}
          ><Icon name="trash" /></button
        >
      {/if}
    </div>
  </div>
{/snippet}

<section class="printer-queue" aria-label={`${printer.name}のキュー`}>
  <div class="printer-heading">
    <h2>{printer.name}</h2>
    <a
      class="icon-btn"
      href={`/printers/${printerId}/control`}
      aria-label="本体の操作"
      title="本体の操作"><Icon name="sliders" /></a
    >
    <a
      bind:this={settingsLink}
      class="icon-btn"
      href={`/printers/${printerId}`}
      aria-label="プリンターの設定"
      title="プリンターの設定"><Icon name="settings" /></a
    >
  </div>
  {#if queue?.printer.synchronized && liveLine(queue.printer.live)}<p
      class="caption live-line"
    >
      {liveLine(queue.printer.live)}
    </p>{/if}
  {#if !menu}{@render issue()}{/if}
  {#if notice}<p class="queue-notice" role="status">{notice}</p>{/if}
  {#if queue}
    {@const reported = queueFailure(queue.current, queue.printer)}
    {#if queue.printer.connection !== "connected" || !queue.printer.synchronized}<p
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
          {@render summary(queue.current)}
          {@render details(queue.current)}
        </details>{/key}
      {#if queue.current.state === "printing"}
        {@const can = jobControls(queue.printer.print.state)}
        <div class="print-controls" role="group" aria-label="印刷の操作">
          {#if can.resume}<button
              class="icon-btn large"
              aria-label="再開"
              title="再開"
              disabled={controlling}
              onclick={() => void printControl("resume")}
              ><Icon name="play" /></button
            >{:else}<button
              class="icon-btn large"
              aria-label="一時停止"
              title="一時停止"
              disabled={controlling || !can.pause}
              onclick={() => void printControl("pause")}
              ><Icon name="pause" /></button
            >{/if}
          <button
            class={stopArmed ? "btn danger" : "icon-btn large"}
            aria-label={stopArmed ? "停止を確定" : "停止"}
            title="停止"
            disabled={controlling || !can.stop}
            onclick={() => void printControl("stop")}
            >{#if stopArmed}停止を確定{:else}<Icon name="square" />{/if}</button
          >
          {#if controlNotice}<span class="caption" role="status"
              >{controlNotice}</span
            >{/if}
        </div>
      {/if}
    {/if}
    {#if reported}
      {@const line = errorLine(reported.failure)}
      <p class="failure device-failure" role="status">
        <span class="tag">{reported.current ? "現在" : "前回"}</span>
        <strong>{line.text}</strong>
        {#if line.help}<a
            class="icon-btn"
            href={line.help}
            target="_blank"
            rel="noopener noreferrer"
            aria-label="公式のエラー解説（別タブ）"
            title="公式のエラー解説"><Icon name="external-link" /></a
          >{/if}
      </p>
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
            })}>再印刷</button
        >
        <button
          class="btn"
          disabled={disabled || !queue.allowed.discard}
          onclick={() =>
            void send({
              type: "discard",
              expected_job: queue!.current!.id,
              cleared: true,
            })}>除外</button
        >
      </div>
      {#if !reported?.current && queue.recovery?.retry_reason}
        <p class="failure" role="status">
          <span>{failureMessage(queue.recovery.retry_reason)}</span>
          {#if !queue.current.plate_deleted}<a
              href={`/plates/${queue.current.plate_id}?edit=1`}
              >プレートの条件を編集</a
            >{/if}
        </p>
      {/if}
      {#if !reported?.current && queue.recovery?.discard_reason && queue.recovery.discard_reason !== queue.recovery.retry_reason}
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
              })}>{queue.current ? "次を印刷" : "印刷"}</button
          >
        </div>
        {#if !queue.allowed.next && queue.recovery?.next_reason}<p
            class="failure"
            role="status"
          >
            {failureMessage(queue.recovery.next_reason)}
          </p>{/if}
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
            oncontextmenu={(e) => openMenu(e, job)}
            onkeydown={(e) => handleKey(e, job)}><Icon name="menu" /></button
          >
          <details id={`job-${job.id}`} bind:open={expanded[job.id]}>
            {@render summary(job)}
            {@render details(job)}
          </details>
        </li>
      {/each}
    </ol>
  {:else if !readError}<p class="state" role="status">
      キューを読み込んでいます…
    </p>{/if}
</section>

{#if menu}
  <Modal title={menu.name} onclose={() => void closeMenu()} dismissible={!busy}>
    <div class="queue-menu">
      <button
        class="btn"
        disabled={disabled || !!reasons.edit}
        onclick={() => {
          if (menuJob) location.assign(`/plates/${menuJob.plate_id}?edit=1`);
        }}>プレート編集</button
      >
      <button
        class="btn"
        disabled={disabled || !!reasons.duplicate}
        onclick={() => void duplicate()}>キュー複製</button
      >
      <button
        class="btn danger"
        disabled={disabled || !!reasons.remove}
        onclick={() => {
          if (menuJob) void send({ type: "remove", job_id: menuJob.id });
        }}>キュー削除</button
      >
      {#each [...new Set([reasons.edit, reasons.duplicate, reasons.remove].filter(Boolean))] as reason}<p
          class="caption"
        >
          {reason}
        </p>{/each}
      {#if busy}<p role="status">キューを更新しています…</p>{/if}
      {@render issue()}
    </div>
  </Modal>
{/if}

<style lang="sass">
  .queue-menu
    display: grid
    gap: var(--sp-2)
    > .btn
      width: 100%
      min-height: 44px
    p
      margin: 0
  .queue-notice
    font-size: var(--fs-sm)
    color: var(--c-muted)

  .printer-queue
    margin-bottom: var(--sp-5)
  .print-controls
    display: flex
    flex-wrap: wrap
    align-items: center
    gap: var(--sp-2)
  .live-line
    margin: 0
  .printer-heading
    display: flex
    align-items: center
    gap: var(--sp-3)
    margin-bottom: var(--sp-3)
    h2
      margin: 0 auto 0 0
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
  .device-failure
    display: flex
    flex-wrap: wrap
    align-items: center
    gap: var(--sp-2)
    margin: var(--sp-3) 0
    padding-left: var(--sp-3)
    border-left: 2px solid var(--c-danger)
    overflow-wrap: anywhere
    .tag
      font-size: var(--fs-xs)
      color: var(--c-muted)
  .actions .btn
    min-height: 44px
    white-space: normal
  .row-actions
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)
</style>
