<script lang="ts">
  import { onMount } from "svelte";
  import { request, type Plate, type Filament } from "./api";
  import { estimateText, failureText, type Estimate } from "./queue";
  let {
    plate,
    filaments,
    edit,
  }: { plate: Plate; filaments: Filament[]; edit: () => void } = $props();
  let estimate = $state<Estimate>(),
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
  const materialError = $derived(
    estimate?.error?.includes("material") ||
      estimate?.error?.includes("filament"),
  );
  async function refresh() {
    if (reading || retrying) return;
    reading = true;
    try {
      const value = await request<Estimate>(`/api/plates/${plate.id}/slice`, {
        signal: controller.signal,
      });
      if (!controller.signal.aborted) {
        estimate = value;
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
        estimate = { state: "pending", seconds: null, error: null };
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

<div aria-label="プレートの試算" class="plate-slice">
  {#if estimate?.state === "failed"}
    <details>
      <summary class="caption">{estimateText(estimate)}</summary>
      <p role="alert">{failureText[estimate.error ?? ""] ?? estimate.error}</p>
      <div class="actions">
        <button class="btn" onclick={edit}>プレートの条件を編集</button>
        {#if materialError}
          {#each materialIds as id}
            <a
              class="btn"
              href={`/filaments/${id}?machine=${encodeURIComponent(plate.conditions.required_machine_profile_key ?? "")}`}
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
    <p class="caption" role="status">
      {estimate ? estimateText(estimate) : "試算を確認中…"}
    </p>
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
