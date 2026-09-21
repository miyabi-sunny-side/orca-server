<script lang="ts">
  import { onMount } from "svelte";
  import type { createThreeViewer } from "./three-viewer";
  let {
    url,
    name,
    passive = false,
  }: { url: string; name: string; passive?: boolean } = $props();
  let mount: HTMLDivElement;
  let viewer = $state<ReturnType<typeof createThreeViewer>>();
  let error = $state("");
  let dimensions = $state("");
  let loading = $state(true);
  let attempt = $state(0);
  onMount(() => {
    let gone = false;
    void import("./three-viewer")
      .then(({ createThreeViewer }) => {
        if (!gone) viewer = createThreeViewer(mount, !passive);
      })
      .catch(() => {
        if (!gone) {
          error = "3D表示を利用できません。WebGLの設定を確認してください。";
          loading = false;
        }
      });
    return () => {
      gone = true;
      viewer?.destroy();
    };
  });
  $effect(() => {
    const current = viewer;
    const target = url;
    attempt;
    if (!current) return;
    loading = true;
    dimensions = "";
    error = "";
    void current
      .loadModel(target)
      .then((result) => {
        if (result.kind === "success") {
          dimensions = result.dimensions;
          loading = false;
        }
      })
      .catch(() => {
        error =
          "STLを表示できません。ファイルや接続を確認して、読み直してください。";
        loading = false;
      });
    return () => current.cancelLoad();
  });
</script>

<aside class="preview" class:passive aria-label="STLプレビュー">
  {#if !passive}<h2>{name}</h2>{/if}
  <div class="viewport" bind:this={mount}></div>
  {#if loading}<p role="status">モデルを読み込んでいます…</p>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
  {#if dimensions}<p class="dimensions" role="status">{dimensions}</p>{/if}
  {#if !passive}<div class="actions">
      <button class="btn" disabled={!dimensions} onclick={() => viewer?.fit()}
        >全体を表示</button
      >
      <button
        class="btn"
        disabled={!dimensions}
        onclick={() => viewer?.zoom(0.8)}>拡大</button
      >
      <button
        class="btn"
        disabled={!dimensions}
        onclick={() => viewer?.zoom(1.25)}>縮小</button
      >
      {#if viewer}<button class="btn" onclick={() => attempt++}>読み直す</button
        >{/if}
    </div>
    <p class="caption">
      ドラッグで回転、ホイールで拡大縮小。表示はモデル単体です。配置・スライス結果ではありません。
    </p>{/if}
</aside>

<style lang="sass">
  .preview
    min-width: 0
    h2
      font-size: var(--fs-xl)
      overflow-wrap: anywhere
      margin: 0 0 var(--sp-3)
    p
      margin: var(--sp-3) 0
    [role="alert"]
      color: var(--c-danger)
  .viewport
    height: clamp(260px, 50vw, 480px)
    width: 100%
    background: var(--c-surface-raised)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    overflow: hidden
    :global(canvas)
      display: block
      touch-action: none
      max-width: 100%
  .preview.passive
    height: 100%
    display: grid
    grid-template-rows: minmax(0, 1fr) auto
    .viewport
      height: 100%
      min-height: 0
    p
      font-size: var(--fs-xs)
      margin: var(--sp-1) 0 0
  .dimensions
    font-variant-numeric: tabular-nums
</style>
