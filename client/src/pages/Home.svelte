<script lang="ts">
  import { onMount } from "svelte";
  import { checkHealth } from "../lib/api";

  let state = $state<"loading" | "success" | "error">("loading");
  const controller = new AbortController();

  async function load() {
    state = "loading";
    try {
      await checkHealth(controller.signal);
      if (!controller.signal.aborted) state = "success";
    } catch {
      if (!controller.signal.aborted) state = "error";
    }
  }

  onMount(() => {
    void load();
    return () => controller.abort();
  });
</script>

<section class="content" data-state={state} aria-label="接続状況">
  {#if state === "loading"}
    <p class="state" role="status">
      <span class="spinner" aria-hidden="true"></span>接続を確認しています…
    </p>
  {:else if state === "error"}
    <div class="state-wrap">
      <p class="state error" role="alert">接続できませんでした</p>
      <button class="btn" type="button" onclick={() => void load()}
        >再試行</button
      >
    </div>
  {:else}
    <p class="state" role="status">OrcaServerに接続しました</p>
  {/if}
</section>
