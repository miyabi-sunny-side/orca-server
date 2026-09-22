<script lang="ts">
  import { onMount } from "svelte";
  import { ApiError, request, type Plate, type Printer } from "./api";
  import { choosePrinter, destinations, emptyConditions } from "./plate";
  import { failureText, type Command, type QueueState } from "./queue";
  let {
    plate = $bindable(),
    paused = false,
    label = "印刷キューへ",
    onbusy,
    onadded,
  }: {
    plate: Plate;
    paused?: boolean;
    label?: string;
    onbusy?: (blocked: boolean) => void;
    onadded?: () => void;
  } = $props();
  const id = $derived(plate.id);
  let printers = $state<Printer[]>([]),
    printerId = $state("");
  let queue = $state<QueueState>(),
    readError = $state(""),
    reading = $state(false),
    busy = $state(false),
    notice = $state(""),
    error = $state("");
  let pending = $state<{ printerId: string; command: Command }>();
  $effect(() => {
    onbusy?.(busy || !!pending);
  });
  const conditions = $derived(plate?.conditions ?? emptyConditions);
  const candidates = $derived(
    destinations(printers, conditions.required_machine_profile_key),
  );
  const importHold = $derived(
    plate.imported &&
      plate.models.some((m) => m.id === plate.imported?.model_id)
      ? plate.imported.selection.print_reason
      : null,
  );
  const hold = $derived(
    importHold
      ? importHold
      : !conditions.required_machine_profile_key
        ? "プレートの印刷条件を設定してください。"
        : !candidates.length
          ? "要求する機種・ノズルに一致するプリンターがありません。"
          : queue?.admission?.reason
            ? (failureText[queue.admission.reason] ?? queue.admission.reason)
            : "",
  );
  const disabled = $derived(
    !!importHold ||
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
  async function refresh() {
    if (reading || busy || !plate || (paused && !pending)) return;
    reading = true;
    const ticket = ++sequence;
    try {
      const p = await request<Printer[]>("/api/printers", {
        signal: controller.signal,
      });
      if (controller.signal.aborted || ticket !== sequence) return;
      printers = p;
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
      onadded?.();
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
    void refresh();
    const timer = setInterval(() => {
      if (!document.hidden) void refresh();
    }, 5000);
    return () => {
      clearInterval(timer);
      controller.abort();
    };
  });
</script>

<div class="queue-add" hidden={paused && !pending}>
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
      >{label}</button
    >
  </div>
  {#if busy}<p role="status">キューに追加しています…</p>{/if}
  {#if notice}<p role="status">
      {notice}。<a href={`/queue?printer_id=${encodeURIComponent(printerId)}`}
        >キューを見る</a
      >
    </p>{/if}
  {#if pending && !busy}<div class="notice">
      <p>
        追加の結果が不明です。キューを確認し、同じ要求の結果を再確認してください。
      </p>
      <div class="actions">
        <button class="btn" onclick={() => void add()}>同じ要求を再確認</button
        ><a href={`/queue?printer_id=${encodeURIComponent(pending.printerId)}`}
          >キューを見る</a
        >
      </div>
    </div>{/if}
  {#if hold && !pending}<div class="notice">
      <p>{hold}</p>
      {#if !importHold}
        <div class="actions">
          {#if printerId}<a href={`/printers/${printerId}/ams`}>AMSを確認</a
            >{:else}<a href="/printers">プリンターを確認</a>{/if}
        </div>
      {/if}
    </div>{/if}
  {#if readError}<div class="notice">
      <p role="alert">{readError}</p>
      <button class="btn" onclick={() => void refresh()}
        >追加条件を読み直す</button
      >
    </div>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
</div>

<style lang="sass">
  .queue-add:not([hidden])
    display: grid
    gap: var(--sp-2)
  p
    margin: 0
</style>
