<script lang="ts">
  import { tick } from "svelte";
  import { request, type Plate } from "../lib/api";
  import { contextMenu } from "../lib/context-menu";
  import Icon from "../lib/Icon.svelte";
  import Modal from "../lib/Modal.svelte";
  import PlateQueueAdd from "../lib/PlateQueueAdd.svelte";
  // One page serves both lists; the archived one keeps the same search and rows.
  const archived = new URLSearchParams(location.search).has("archived");
  let query = $state("");
  let revision = $state(0);
  let plates = $state<Plate[]>([]);
  let phase = $state<"loading" | "error" | "success">("loading");
  let error = $state("");
  let search = $state<HTMLInputElement>();
  let list = $state<HTMLUListElement>();
  let menuPlate = $state<Plate>(),
    menuRow: HTMLAnchorElement | undefined;
  let queueBusy = $state(false),
    deleting = $state(false),
    archiving = $state(false),
    confirmDelete = $state(false),
    menuError = $state(""),
    notice = $state("");
  let cancelButton = $state<HTMLButtonElement>(),
    deleteButton = $state<HTMLButtonElement>(),
    archiveButton = $state<HTMLButtonElement>();
  const busy = $derived(queueBusy || deleting || archiving);
  let duplicateName = $state("");
  let confirmDuplicate = $state(false),
    duplicating = $state(false);
  let duplicateButton = $state<HTMLButtonElement>(),
    nameInput = $state<HTMLInputElement>();
  function openMenu(row: HTMLElement, plate: Plate) {
    menuRow = row as HTMLAnchorElement;
    menuPlate = plate;
    menuError = "";
    queueBusy = false;
    confirmDelete = false;
    confirmDuplicate = false;
  }
  async function closeMenu() {
    if (deleting || duplicating || archiving) return;
    menuPlate = undefined;
    confirmDelete = false;
    confirmDuplicate = false;
    await tick();
    (menuRow?.isConnected
      ? menuRow
      : (list?.querySelector("a") ?? search)
    )?.focus();
  }
  async function askToDelete() {
    if (!menuPlate || busy) return;
    confirmDelete = true;
    menuError = "";
    await tick();
    cancelButton?.focus();
  }
  async function cancelDelete() {
    if (deleting) return;
    confirmDelete = false;
    menuError = "";
    await tick();
    deleteButton?.focus();
  }
  async function remove() {
    if (!menuPlate || !confirmDelete || deleting || queueBusy) return;
    const plate = menuPlate;
    deleting = true;
    menuError = "";
    try {
      await request(`/api/plates/${plate.id}`, { method: "DELETE" });
      plates = plates.filter((p) => p.id !== plate.id);
      notice = `「${plate.name}」を削除しました`;
      deleting = false;
      await closeMenu();
    } catch (cause) {
      menuError = (cause as Error).message;
      deleting = false;
      await tick();
      cancelButton?.focus();
    }
  }

  // Archiving is reversible, so it needs no confirmation.
  async function setArchived(value: boolean) {
    if (!menuPlate || busy) return;
    const plate = menuPlate;
    archiving = true;
    menuError = "";
    try {
      await request(`/api/plates/${plate.id}/archive`, {
        method: value ? "PUT" : "DELETE",
      });
      plates = plates.filter((p) => p.id !== plate.id);
      notice = `「${plate.name}」を${value ? "アーカイブ" : "復元"}しました`;
      archiving = false;
      await closeMenu();
    } catch (cause) {
      menuError = (cause as Error).message;
      archiving = false;
      await tick();
      archiveButton?.focus();
    }
  }

  async function askToDuplicate() {
    if (!menuPlate || queueBusy) return;
    duplicateName = menuPlate.name;
    confirmDuplicate = true;
    menuError = "";
    await tick();
    nameInput?.focus();
  }
  async function cancelDuplicate() {
    if (duplicating) return;
    confirmDuplicate = false;
    menuError = "";
    await tick();
    duplicateButton?.focus();
  }
  async function duplicate(event: SubmitEvent) {
    event.preventDefault();
    if (!menuPlate || duplicating) return;
    duplicating = true;
    menuError = "";
    try {
      const copy = await request<Plate>(
        `/api/plates/${menuPlate.id}/duplicate`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: duplicateName.trim() }),
        },
      );
      location.assign(`/plates/${copy.id}?edit=1`);
    } catch (cause) {
      menuError = (cause as Error).message;
      duplicating = false;
      await tick();
      nameInput?.focus();
    }
  }

  $effect(() => {
    const q = query.trim();
    revision;
    const controller = new AbortController();
    phase = "loading";
    const timer = setTimeout(async () => {
      try {
        const result = await request<Plate[]>(
          `/api/plates?q=${encodeURIComponent(q)}${archived ? "&archived=true" : ""}`,
          { signal: controller.signal },
        );
        if (!controller.signal.aborted) {
          plates = result;
          phase = "success";
        }
      } catch (cause) {
        if (!controller.signal.aborted) {
          error = (cause as Error).message;
          phase = "error";
        }
      }
    }, 150);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  });

  function move(event: KeyboardEvent) {
    if (!["ArrowUp", "ArrowDown"].includes(event.key)) return;
    event.preventDefault();
    const row = (event.currentTarget as HTMLElement).closest("li");
    const target =
      event.key === "ArrowDown"
        ? row?.nextElementSibling
        : row?.previousElementSibling;
    const link = target?.querySelector("a");
    if (link) link.focus();
    else if (event.key === "ArrowUp") search?.focus();
  }
</script>

<section class="content" aria-label="プレート一覧" data-state={phase}>
  {#if notice}<p role="status">{notice}</p>{/if}
  <div class="page-heading">
    {#if archived}
      <div class="archive-heading">
        <a
          class="icon-btn"
          href="/plates"
          aria-label="プレート一覧へ戻る"
          title="プレート一覧へ戻る"><Icon name="arrow-left" /></a
        >
        <h1>アーカイブ</h1>
      </div>
    {:else}
      <h1>プレート</h1>
      <div class="create-actions">
        <a
          class="import-link"
          aria-label="ファイルから取り込む"
          href="/plates/new?source=file">ファイル取込</a
        ><a class="btn primary" href="/plates/new">新規作成</a>
      </div>
    {/if}
  </div>
  <div class="search-row">
    <label class="field" for="plate-search">
      <span>名前・モデル名で検索</span>
      <input
        id="plate-search"
        type="search"
        bind:value={query}
        bind:this={search}
        placeholder="例: box、机"
        onkeydown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            list?.querySelector("a")?.focus();
          }
        }}
      />
    </label>
    {#if !archived}
      <a
        class="icon-btn large"
        href="/plates?archived=1"
        aria-label="アーカイブ済み"
        title="アーカイブ済み"><Icon name="archive" /></a
      >
    {/if}
  </div>
  {#if phase === "loading"}
    <p class="state" role="status">
      <span class="spinner" aria-hidden="true"
      ></span>プレートを読み込んでいます…
    </p>
  {:else if phase === "error"}
    <div class="notice">
      <p role="alert">{error}</p>
      <button class="btn" onclick={() => revision++}>再試行</button>
    </div>
  {:else if plates.length === 0}
    <p class="state">
      {query.trim()
        ? "一致するプレートがありません"
        : archived
          ? "アーカイブ済みのプレートはありません"
          : "保存済みプレートはありません"}
    </p>
  {:else}
    <ul class="plate-list" bind:this={list}>
      {#each plates as plate (plate.id)}
        <li>
          <a
            class="plate-row"
            aria-haspopup="dialog"
            href={`/plates/${plate.id}`}
            use:contextMenu={(row) => openMenu(row, plate)}
            onkeydown={(event) => {
              if (!event.defaultPrevented) move(event);
            }}
          >
            <strong>{plate.name}</strong>
            <span class="caption"
              >{plate.models.length}モデル · {plate.models.reduce(
                (sum, model) => sum + model.quantity,
                0,
              )}個</span
            >
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if menuPlate}
  <Modal
    title={confirmDelete
      ? "プレートを削除"
      : confirmDuplicate
        ? "プレートを複製"
        : menuPlate.name}
    dismissible={!deleting && !duplicating && !archiving}
    onclose={() =>
      void (confirmDelete
        ? cancelDelete()
        : confirmDuplicate
          ? cancelDuplicate()
          : closeMenu())}
  >
    {#if confirmDuplicate}
      <form onsubmit={duplicate}>
        <label class="field"
          ><span>プレート名</span><input
            bind:this={nameInput}
            bind:value={duplicateName}
            required
            maxlength="256"
            disabled={duplicating}
          /></label
        >
        {#if menuError}<p role="alert">{menuError}</p>{/if}
        <div class="actions">
          <button class="btn primary" type="submit" disabled={duplicating}
            >{duplicating ? "複製中…" : "複製"}</button
          >
          <button
            class="btn"
            type="button"
            disabled={duplicating}
            onclick={() => void cancelDuplicate()}>キャンセル</button
          >
        </div>
      </form>
    {:else if confirmDelete}
      <p class="delete-name">「{menuPlate.name}」を削除しますか？</p>
      <p class="caption">
        保存済み一覧から削除します。追加済みのキューは残ります。
      </p>
      <div class="actions">
        <button
          class="btn"
          bind:this={cancelButton}
          disabled={deleting}
          onclick={() => void cancelDelete()}>キャンセル</button
        >
        <button
          class="btn danger"
          disabled={deleting}
          onclick={() => void remove()}>{deleting ? "削除中…" : "削除"}</button
        >
      </div>
      {#if menuError}<p role="alert">{menuError}</p>{/if}
    {:else}
      <div class="plate-menu">
        {#if archived}
          <button
            class="btn"
            bind:this={archiveButton}
            disabled={busy}
            onclick={() => void setArchived(false)}
            >{archiving ? "復元中…" : "復元"}</button
          >
        {:else}
          <PlateQueueAdd
            bind:plate={menuPlate}
            label="キュー追加"
            paused={deleting || archiving}
            onbusy={(value) => (queueBusy = value)}
            onadded={() => {
              notice = "キューに追加しました";
              void closeMenu();
            }}
          />
          <button
            class="btn"
            disabled={busy}
            onclick={() => location.assign(`/plates/${menuPlate!.id}?edit=1`)}
            >編集</button
          >
          <button
            class="btn"
            bind:this={duplicateButton}
            disabled={busy}
            onclick={() => void askToDuplicate()}>複製</button
          >
          <button
            class="btn"
            bind:this={archiveButton}
            disabled={busy}
            onclick={() => void setArchived(true)}
            >{archiving ? "アーカイブ中…" : "アーカイブ"}</button
          >
        {/if}
        <button
          class="btn danger"
          bind:this={deleteButton}
          disabled={busy}
          onclick={() => void askToDelete()}>削除</button
        >
        {#if menuError}<p role="alert">{menuError}</p>{/if}
      </div>
    {/if}
  </Modal>
{/if}

<style lang="sass">
  .create-actions, .archive-heading
    display: flex
    align-items: center
    gap: var(--sp-2)
  // The archived-list link shares the search row so it never pushes the list down.
  .search-row
    display: flex
    align-items: flex-end
    gap: var(--sp-2)
    margin-bottom: var(--sp-3)
    .field
      flex: 1
      margin-bottom: 0
  .import-link
    font-size: var(--fs-sm)
    min-height: 44px
    display: flex
    align-items: center

  .plate-menu
    display: grid
    gap: var(--sp-2)
    :global(.queue-add > .actions)
      display: grid
      margin: 0
  .plate-menu :global(.btn), form .btn
    min-height: 44px
  .delete-name
    overflow-wrap: anywhere
</style>
