<script lang="ts">
  import PlateEditor from "../lib/PlateEditor.svelte";
</script>

<svelte:head><title>新規作成 · OrcaServer</title></svelte:head>
<section class="content" aria-label="新しいプレート">
  <h1>新しいプレート</h1>
  <PlateEditor
    saved={(plate) => window.location.assign(`/plates/${plate.id}`)}
  />
  <div class="actions"><a href="/plates">プレート一覧へ</a></div>
</section>
