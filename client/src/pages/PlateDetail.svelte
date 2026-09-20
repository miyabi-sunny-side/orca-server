<script lang="ts">
  import { onMount } from "svelte";
  import { request, type Plate } from "../lib/api";
  import PlateEditor from "../lib/PlateEditor.svelte";
  let { id }: { id: string } = $props();
  let plate = $state<Plate>(),
    loading = $state(true),
    error = $state(""),
    editing = $state(false);
  const controller = new AbortController();
  async function load() {
    loading = true;
    error = "";
    try {
      const value = await request<Plate>(`/api/plates/${id}`, {
        signal: controller.signal,
      });
      if (!controller.signal.aborted) plate = value;
    } catch (e) {
      if (!controller.signal.aborted) error = (e as Error).message;
    } finally {
      if (!controller.signal.aborted) loading = false;
    }
  }
  onMount(() => {
    void load();
    return () => controller.abort();
  });
</script>

<svelte:head
  ><title>{plate?.name ?? "プレート"} · OrcaServer</title></svelte:head
>
<section class="content" aria-label="プレート詳細">
  <a href="/">プレート一覧へ</a>
  {#if loading}<p class="state" role="status">プレートを読み込んでいます…</p>
  {:else if plate}
    <div class="page-heading"><h1>{plate.name}</h1></div>
    {#if editing}
      <PlateEditor
        initial={plate}
        saved={(value) => {
          plate = value;
          editing = false;
        }}
        cancel={() => {
          editing = false;
          void load();
        }}
      />
    {:else}
      <div class="actions">
        <a class="btn primary" href={`/queue?plate=${plate.id}`}>印刷キューへ</a
        ><button class="btn" onclick={() => (editing = true)}>構成を編集</button
        >
      </div>
      <ul class="plate-list">
        {#each plate.models as model}<li class="plate-row">
            <strong>{model.name}</strong><span>{model.quantity}個</span>
            {#if model.source}<span class="caption"
                >SCAD参照: {model.source}</span
              >
            {:else}<a
                href={`/api/plates/${plate.id}/files/${model.id}`}
                download={model.name}>アップロードした元STLを取得</a
              >{/if}
          </li>{/each}
      </ul>
      <p class="caption">
        SCADモデルは印刷準備の開始時に最新データを取得します。材料と印刷条件はキューで選びます。構成の編集は次の準備から反映されます。
      </p>
    {/if}
  {/if}
  {#if error}<div class="notice">
      <p role="alert">{error}</p>
      <button class="btn" onclick={() => void load()}>読み直す</button>
    </div>{/if}
</section>
