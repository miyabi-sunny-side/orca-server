<script lang="ts">
  import FileImport from "../lib/FileImport.svelte";
  const fromFile =
    new URLSearchParams(location.search).get("source") === "file";
  import PlateEditor from "../lib/PlateEditor.svelte";
</script>

<svelte:head><title>新規作成 · OrcaServer</title></svelte:head>
<section class="content" class:fromFile aria-label="新しいプレート">
  <h1>新しいプレート</h1>
  <nav class="actions" aria-label="モデルの取得元">
    <a
      class="btn"
      aria-current={fromFile ? "page" : undefined}
      href="/plates/new?source=file">ファイルから取り込む</a
    >
    <a
      class="btn"
      aria-current={!fromFile ? "page" : undefined}
      href="/plates/new">SCADから選ぶ</a
    >
  </nav>
  {#if fromFile}<FileImport />{:else}<PlateEditor
      saved={(plate) => window.location.assign(`/plates/${plate.id}`)}
    />{/if}
  <div class="actions"><a href="/plates">プレート一覧へ</a></div>
</section>

<style lang="sass">
  .fromFile
    max-width: 1200px
  nav.actions
    margin-bottom: var(--sp-4)
  a[aria-current="page"]
    border-color: var(--c-primary)
    background: var(--c-hover-1)
</style>
