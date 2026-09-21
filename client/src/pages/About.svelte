<script lang="ts">
  import { onMount } from "svelte";
  import { request } from "../lib/api";
  let info = $state<{ version: string; source_url: string | null }>();
  let error = $state("");
  const controller = new AbortController();
  async function load() {
    error = "";
    try {
      const result = await request<{
        version: string;
        source_url: string | null;
      }>("/api/about", { signal: controller.signal });
      if (!controller.signal.aborted) info = result;
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
    }
  }
  onMount(() => {
    void load();
    return () => controller.abort();
  });
</script>

<svelte:head><title>ライセンスとソース · OrcaServer</title></svelte:head>
<section class="content" aria-label="ライセンスとソース">
  <h1>OrcaServer{info ? ` ${info.version}` : ""}</h1>
  <p>Copyright © 2026 miyabi-sunny-side contributors</p>
  <p>
    OrcaServerはGNU Affero General Public License version
    3（AGPL-3.0-only）で提供します。このライセンスに従って利用・改変・再配布できます。無保証です。
  </p>
  <p>
    <a href="/LICENSE">ライセンス本文</a> ·
    <a href="/THIRD_PARTY_NOTICES">第三者の著作権・許諾表示</a>
  </p>
  {#if error}
    <div class="notice">
      <p role="alert">{error}</p>
      <button class="btn" onclick={() => void load()}>再試行</button>
    </div>
  {:else if !info}
    <p class="state" role="status">ビルド情報を読み込んでいます…</p>
  {:else if info.source_url}
    <p><a href={info.source_url}>このビルドのソースを取得</a></p>
    <p>ビルド手順はソース内のdocs/development.mdにあります。</p>
  {:else}
    <p>
      このビルドにはソースの公開先が設定されていません。配布者へ確認してください。
    </p>
  {/if}
  <a href="/plates">プレート一覧へ</a>
</section>

<style lang="sass">
  p, a
    overflow-wrap: anywhere
</style>
