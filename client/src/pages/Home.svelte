<script lang="ts">
  import { tick, onDestroy } from "svelte";
  import { request, type Plate } from "../lib/api";
  import Modal from "../lib/Modal.svelte";
  import PlateQueueAdd from "../lib/PlateQueueAdd.svelte";
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
    confirmDelete = $state(false),
    menuError = $state(""),
    notice = $state("");
  let cancelButton = $state<HTMLButtonElement>(),
    deleteButton = $state<HTMLButtonElement>();
  let duplicateName = $state("");
  let confirmDuplicate = $state(false),
    duplicating = $state(false);
  let duplicateButton = $state<HTMLButtonElement>(),
    nameInput = $state<HTMLInputElement>();
  let press: ReturnType<typeof setTimeout> | undefined,
    point = { x: 0, y: 0 },
    longPressed = false;
  function cancelPress() {
    clearTimeout(press);
    press = undefined;
  }
  onDestroy(cancelPress);
  function openMenu(event: Event, plate: Plate) {
    event.preventDefault();
    cancelPress();
    menuRow = event.currentTarget as HTMLAnchorElement;
    menuPlate = plate;
    menuError = "";
    queueBusy = false;
    confirmDelete = false;
    confirmDuplicate = false;
  }
  async function closeMenu() {
    if (deleting || duplicating) return;
    menuPlate = undefined;
    confirmDelete = false;
    confirmDuplicate = false;
    await tick();
    (menuRow?.isConnected
      ? menuRow
      : (list?.querySelector("a") ?? search)
    )?.focus();
  }
  function startPress(event: PointerEvent, plate: Plate) {
    cancelPress();
    longPressed = false;
    if (event.pointerType !== "touch" && event.pointerType !== "pen") return;
    point = { x: event.clientX, y: event.clientY };
    const row = event.currentTarget as HTMLAnchorElement;
    press = setTimeout(() => {
      longPressed = true;
      menuRow = row;
      menuPlate = plate;
      menuError = "";
      queueBusy = false;
      confirmDelete = false;
      confirmDuplicate = false;
    }, 500);
  }
  async function askToDelete() {
    if (!menuPlate || queueBusy || deleting) return;
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
          `/api/plates?q=${encodeURIComponent(q)}`,
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
    <h1>プレート</h1>
    <div class="create-actions">
      <a
        class="import-link"
        aria-label="ファイルから取り込む"
        href="/plates/new?source=file">ファイル取込</a
      ><a class="btn primary" href="/plates/new">新規作成</a>
    </div>
  </div>
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
            oncontextmenu={(event) => openMenu(event, plate)}
            onkeydown={(event) => {
              if (
                event.key === "ContextMenu" ||
                (event.shiftKey && event.key === "F10")
              )
                openMenu(event, plate);
              else move(event);
            }}
            onpointerdown={(event) => startPress(event, plate)}
            onpointermove={(event) => {
              if (
                Math.hypot(event.clientX - point.x, event.clientY - point.y) >
                10
              )
                cancelPress();
            }}
            onpointerup={cancelPress}
            onpointercancel={cancelPress}
            onpointerleave={cancelPress}
            onclick={(event) => {
              if (longPressed) {
                event.preventDefault();
                longPressed = false;
              }
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
    dismissible={!deleting && !duplicating}
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
        <PlateQueueAdd
          bind:plate={menuPlate}
          label="キュー追加"
          paused={deleting}
          onbusy={(value) => (queueBusy = value)}
          onadded={() => {
            notice = "キューに追加しました";
            void closeMenu();
          }}
        />
        <button
          class="btn"
          disabled={queueBusy || deleting}
          onclick={() => location.assign(`/plates/${menuPlate!.id}?edit=1`)}
          >編集</button
        >
        <button
          class="btn"
          bind:this={duplicateButton}
          disabled={queueBusy || deleting}
          onclick={() => void askToDuplicate()}>複製</button
        >
        <button
          class="btn danger"
          bind:this={deleteButton}
          disabled={queueBusy || deleting}
          onclick={() => void askToDelete()}>削除</button
        >
        {#if menuError}<p role="alert">{menuError}</p>{/if}
      </div>
    {/if}
  </Modal>
{/if}

<style lang="sass">
  .create-actions
    display: flex
    align-items: center
    gap: var(--sp-2)
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
