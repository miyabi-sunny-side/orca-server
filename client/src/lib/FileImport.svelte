<script lang="ts">
  import { onDestroy } from "svelte";
  import { request, type ImportedSelection } from "./api";
  import { uploadBody, type UploadDraft } from "./file-import";
  import PlateEditor from "./PlateEditor.svelte";
  import StlPreview from "./StlPreview.svelte";
  let files = $state<File[]>([]);
  let plates = $state<ImportedSelection[]>([]);
  let selected = $state(0);
  let displayed = $state(0);
  let included = $state<number[]>([]);
  let draft = $state<UploadDraft>();
  let busy = $state(false);
  let saving = $state(false);
  let error = $state("");
  let phase = $state<"read" | "preview">("read");
  let controller = new AbortController();
  let previews: string[] = [];
  function clearPreviews() {
    for (const url of previews) URL.revokeObjectURL(url);
    previews = [];
    draft = undefined;
  }
  onDestroy(() => {
    controller.abort();
    clearPreviews();
  });
  async function inspect() {
    controller.abort();
    controller = new AbortController();
    const signal = controller.signal;
    clearPreviews();
    busy = false;
    plates = [];
    error = "";
    selected = 0;
    displayed = 0;
    phase = "read";
    if (!files.length) return;
    if (
      files.length > 64 ||
      files.reduce((sum, f) => sum + f.size, 0) > 64 * 1024 * 1024
    ) {
      error = "64ファイル・合計64 MiB以内で選んでください。";
      return;
    }
    const is3mf = files.some((f) => /\.3mf$/i.test(f.name));
    if (
      files.some((f) => !/\.(stl|3mf)$/i.test(f.name)) ||
      (is3mf && files.length !== 1)
    ) {
      error = "STL（複数可）または3MFを1ファイル選んでください。";
      return;
    }
    busy = true;
    try {
      if (is3mf) {
        plates = await request<ImportedSelection[]>("/api/plates/file-info", {
          method: "POST",
          body: uploadBody(files),
          signal,
        });
        if (signal.aborted) return;
        await preview();
      } else {
        previews = files.map((file) => URL.createObjectURL(file));
        included = files.map((_, index) => index);
        draft = { files, previews };
      }
    } catch (e) {
      if (!signal.aborted) error = (e as Error).message;
    } finally {
      if (!signal.aborted) busy = false;
    }
  }
  async function preview() {
    controller.abort();
    controller = new AbortController();
    const signal = controller.signal;
    clearPreviews();
    error = "";
    phase = "preview";
    busy = true;
    try {
      const response = await fetch("/api/plates/file-preview", {
        method: "POST",
        body: uploadBody(files, selected),
        signal,
      });
      if (!response.ok) {
        const result = await response.json().catch(() => ({}));
        throw new Error(
          result.error ?? "プレビューを取得できません。再試行してください。",
        );
      }
      const blob = await response.blob();
      if (signal.aborted) return;
      previews = [URL.createObjectURL(blob)];
      displayed = 0;
      included = [0];
      draft = { files, plate: selected, selection: plates[selected], previews };
    } catch (e) {
      if (!signal.aborted)
        error =
          e instanceof TypeError
            ? "サーバーに接続できません。再試行してください。"
            : (e as Error).message;
    } finally {
      if (!signal.aborted) busy = false;
    }
  }
</script>

<label class="field"
  ><span>モデルファイル</span><input
    type="file"
    accept=".stl,.3mf"
    multiple
    disabled={saving}
    onchange={(e) => {
      files = Array.from(e.currentTarget.files ?? []);
      void inspect();
    }}
  /></label
>
<p class="caption">STLは複数選べます。3MFは1ファイルずつ取り込みます。</p>
{#if plates.length > 1}
  <label class="field"
    ><span>取り込むプレート</span><select
      disabled={saving}
      bind:value={selected}
      onchange={() => void preview()}
    >
      {#each plates as plate, index}<option value={index}>{plate.name}</option
        >{/each}
    </select></label
  >
{/if}
{#if busy}<p role="status">
    {phase === "read"
      ? "ファイルを確認しています…"
      : "選んだ形状を読み込んでいます…"}
  </p>{/if}
{#if error}<div class="notice">
    <p role="alert">{error}</p>
    <button
      class="btn"
      onclick={() => void (phase === "read" ? inspect() : preview())}
      >再試行</button
    >
  </div>{/if}
{#if draft}
  {#if draft.selection?.print_reason}<p class="notice" role="status">
      {draft.selection.print_reason}
    </p>{/if}
  <div class="import-layout">
    <div class="shape">
      {#if included.length > 1}<label class="field"
          ><span>表示するモデル</span><select bind:value={displayed}>
            {#each included as index}<option value={index}
                >{draft.files[index].name}</option
              >{/each}
          </select></label
        >{/if}
      <StlPreview
        name={draft.selection?.name ?? draft.files[displayed].name}
        url={draft.previews[displayed]}
      />
    </div>
    <div class="controls">
      {#key draft}<PlateEditor
          upload={draft}
          onbusy={(value) => (saving = value)}
          onremovefile={(index) => {
            included = included.filter((i) => i !== index);
            if (displayed === index) displayed = included[0];
          }}
          saved={(plate) => window.location.assign(`/plates/${plate.id}`)}
        />{/key}
    </div>
  </div>
{/if}

<style lang="sass">
  .import-layout
    display: grid
    grid-template-columns: minmax(0, 1fr)
    gap: var(--sp-5)
    > div
      min-width: 0
  .caption
    margin-top: calc(-1 * var(--sp-2))
    margin-bottom: var(--sp-4)
  @media (min-width: 768px)
    .import-layout
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr)
      .controls
        grid-column: 1
        grid-row: 1
      .shape
        grid-column: 2
        grid-row: 1
</style>
