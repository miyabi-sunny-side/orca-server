<script lang="ts">
  import { onMount } from "svelte";
  import { request, type Plate, type Filament } from "./api";
  import { estimateText, failureText, type PrinterEstimate } from "./queue";
  let {
    plate,
    filaments,
    edit,
  }: { plate: Plate; filaments: Filament[]; edit: () => void } = $props();
  let estimates = $state<PrinterEstimate[]>(),
    error = $state(""),
    reading = $state(false),
    retrying = $state(false);
  const controller = new AbortController();
  const materialIds = $derived([
    ...new Set(
      [
        plate.conditions.filament_id,
        plate.conditions.secondary_filament_id,
        plate.conditions.support_enabled
          ? plate.conditions.support_interface_filament_id
          : null,
      ].filter((id): id is string => !!id),
    ),
  ]);
  const materialError = (estimate: PrinterEstimate) =>
    estimate.reason === "material_setting" ||
    estimate.error?.includes("material") ||
    estimate.error?.includes("filament");
  async function refresh() {
    if (reading || retrying) return;
    reading = true;
    try {
      const value = await request<{ printers: PrinterEstimate[] }>(
        `/api/plates/${plate.id}/slice`,
        { signal: controller.signal },
      );
      if (!controller.signal.aborted) {
        estimates = value.printers;
        error = "";
      }
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
    } finally {
      if (!controller.signal.aborted) reading = false;
    }
  }
  async function retry() {
    if (retrying || reading) return;
    retrying = true;
    try {
      await request(`/api/plates/${plate.id}/slice`, {
        method: "POST",
        signal: controller.signal,
      });
      if (!controller.signal.aborted) {
        estimates = estimates?.map((e) => ({
          ...e,
          state: "pending",
          seconds: null,
          error: null,
          reason: null,
        }));
        error = "";
      }
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
    } finally {
      if (!controller.signal.aborted) retrying = false;
    }
    await refresh();
  }
  onMount(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 2000);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  });
</script>

{#snippet result(estimate: PrinterEstimate, named: boolean)}
  {@const text = named
    ? `${estimate.printer_name} · ${estimateText(estimate)}`
    : estimateText(estimate)}
  {#if estimate.state === "failed"}
    <details>
      <summary class="caption">{text}</summary>
      <p role="alert">
        {failureText[estimate.error ?? ""] ?? estimate.error}
      </p>
      <div class="actions">
        <button class="btn" onclick={edit}>プレートの条件を編集</button>
        {#if materialError(estimate)}
          {#each materialIds as id}
            <a
              class="btn"
              href={`/filaments/${id}?machine=${encodeURIComponent(estimate.machine_profile_key)}`}
            >
              {filaments.find((f) => f.id === id)?.name ??
                "フィラメント"}の材料の設定
            </a>
          {/each}
        {/if}
        <button
          class="btn"
          disabled={reading || retrying}
          onclick={() => void retry()}>再試算</button
        >
      </div>
    </details>
  {:else}
    <p class="caption" role="status">{text}</p>
  {/if}
{/snippet}

<div aria-label="プレートの試算" class="plate-slice">
  {#if !estimates}
    <p class="caption" role="status">試算を確認中…</p>
  {:else if !estimates.length}
    <p class="caption">
      プリンターがありません。<a href="/printers/new">プリンターを登録</a>
    </p>
  {:else if estimates.length === 1}
    {@render result(estimates[0], false)}
  {:else}
    <ul aria-label="プリンターごとの試算">
      {#each estimates as estimate (estimate.printer_id)}
        <li>{@render result(estimate, true)}</li>
      {/each}
    </ul>
  {/if}
  {#if error}
    <p role="alert">計算状態を取得できませんでした。{error}</p>
    <button
      class="btn"
      disabled={reading || retrying}
      onclick={() => void refresh()}>計算状態を読み直す</button
    >
  {/if}
</div>

<style lang="sass">
  .plate-slice
    margin: var(--sp-2) 0
    overflow-wrap: anywhere
    p
      margin: var(--sp-1) 0
    ul
      list-style: none
      margin: 0
      padding: 0
    li > p
      min-height: 44px
      align-content: center
      margin: 0
    summary
      cursor: pointer
      min-height: 44px
      align-content: center
    .actions
      display: flex
      gap: var(--sp-2)
      flex-wrap: wrap
    .btn
      min-height: 44px
</style>
