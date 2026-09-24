<script lang="ts">
  import { onMount } from "svelte";
  import { request, type Plate, type Filament } from "../lib/api";
  import StlPreview from "../lib/StlPreview.svelte";
  import PlateEditor from "../lib/PlateEditor.svelte";
  import PlateQueueAdd from "../lib/PlateQueueAdd.svelte";
  import { emptyConditions, materialRoles, roleFields } from "../lib/plate";
  let { id }: { id: string } = $props();
  let plate = $state<Plate>(),
    loading = $state(true),
    error = $state("");
  let editing = $state(new URLSearchParams(location.search).has("edit"));
  let filaments = $state<Filament[]>([]),
    queueBusy = $state(false);
  let selectedModelId = $state("");
  const selectedModel = $derived(
    plate?.models.find((model) => model.id === selectedModelId) ??
      plate?.models[0],
  );
  const conditions = $derived(plate?.conditions ?? emptyConditions);
  const roles = $derived(materialRoles(plate?.models ?? []));
  const controller = new AbortController();
  async function load() {
    loading = true;
    error = "";
    try {
      const [saved, materials] = await Promise.all([
        request<Plate>(`/api/plates/${id}`, { signal: controller.signal }),
        request<Filament[]>("/api/filaments", { signal: controller.signal }),
      ]);
      if (!controller.signal.aborted) {
        plate = saved;
        filaments = materials;
      }
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
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
  <a href="/plates">プレート一覧へ</a>
  {#if loading}<p class="state" role="status">プレートを読み込んでいます…</p>
  {:else if plate}
    <div class="detail-layout" class:editing>
      <div class="controls">
        <div class="page-heading"><h1>{plate.name}</h1></div>
        {#if plate.imported}
          <p>
            <a
              href={`/api/plates/${plate.id}/original`}
              download={plate.imported.file_name}>元の3MFを取得</a
            >
            <span class="caption"
              >{plate.imported.file_name} · {plate.imported.selection
                .name}</span
            >
          </p>
        {/if}
        <PlateQueueAdd
          bind:plate
          paused={editing}
          onbusy={(value) => (queueBusy = value)}
        />
        {#if editing && !queueBusy}
          <PlateEditor
            initial={plate}
            saved={(value) => {
              plate = value;
              editing = false;
              history.replaceState(null, "", `/plates/${id}`);
            }}
            cancel={() => {
              editing = false;
              history.replaceState(null, "", `/plates/${id}`);
              void load();
            }}
          />
        {:else}
          <div class="conditions" aria-label="保存した印刷条件">
            <p>
              {conditions.required_machine_profile_key ?? "機種・ノズル未設定"}
            </p>
            {#each roles as role}<p>
                {#if roles.length > 1 || role === "secondary"}{role}:
                {/if}
                {filaments.find((f) => f.id === conditions[roleFields[role]])
                  ?.name ??
                  (conditions[roleFields[role]]
                    ? "材料を確認中"
                    : "フィラメント未設定")}
              </p>{/each}
            <p>
              {conditions.process_profile_key ?? "工程未設定"} · {conditions.bed_type ??
                "ビルドプレート未設定"}
            </p>
          </div>
          <button
            class="btn"
            disabled={queueBusy}
            onclick={() => {
              editing = true;
            }}>構成を編集</button
          >
          <ul class="plate-list">
            {#each plate.models as model}<li class="plate-row">
                <button
                  class="model-choice"
                  aria-pressed={selectedModel?.id === model.id}
                  onclick={() => (selectedModelId = model.id)}
                >
                  <strong>{model.name}</strong><span>{model.quantity}個</span>
                  <span class="caption"
                    >{selectedModel?.id === model.id
                      ? "表示中"
                      : "形状を見る"}</span
                  >
                </button>
                {#if model.source}<span class="caption"
                    >SCAD参照: {model.source}</span
                  >{:else}<a
                    href={`/api/plates/${plate.id}/files/${model.id}`}
                    download={model.name}
                    >{plate.imported?.model_id === model.id
                      ? "確認用STLを取得"
                      : "アップロードした元STLを取得"}</a
                  >{/if}
              </li>{/each}
          </ul>
          <p class="caption">
            SCADモデルは試算時と印刷開始時に最新データを取得します。条件の編集は未準備の待機分へ反映されます。印刷はキューで手動開始します。
          </p>
        {/if}
      </div>
      {#if !editing && selectedModel}<StlPreview
          name={selectedModel.name}
          url={`/api/plates/${encodeURIComponent(plate.id)}/models/${encodeURIComponent(selectedModel.id)}`}
        />{/if}
    </div>
  {/if}
  {#if error}<div class="notice">
      <p role="alert">{error}</p>
      {#if !queueBusy}<button class="btn" onclick={() => void load()}
          >読み直す</button
        >{/if}
    </div>{/if}
</section>

<style lang="sass">
  section.content
    max-width: 1200px
  .detail-layout
    display: grid
    gap: var(--sp-5)
    grid-template-columns: minmax(0, 1fr)
    .controls
      min-width: 0
    &.editing
      max-width: 720px
      margin: auto
  .model-choice
    display: flex
    flex-direction: column
    align-items: start
    gap: var(--sp-1)
    width: 100%
    font: inherit
    text-align: left
    color: inherit
    background: transparent
    border: 0
    border-radius: var(--radius-sm)
    padding: var(--sp-2)
    cursor: pointer
    overflow-wrap: anywhere
    &[aria-pressed="true"], &:hover
      background: var(--c-hover-1)
  @media (min-width: 768px)
    .detail-layout:not(.editing)
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr)

  .conditions
    margin-bottom: var(--sp-3)
    overflow-wrap: anywhere
    p
      margin: var(--sp-1) 0
    p:not(:first-child)
      color: var(--c-muted)
      font-size: var(--fs-sm)
</style>
