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
    menuError = $state(""),
    notice = $state("");
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
  }
  async function closeMenu() {
    if (deleting) return;
    menuPlate = undefined;
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
    }, 500);
  }
  async function remove() {
    if (!menuPlate || deleting || queueBusy) return;
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
    <a class="btn primary" href="/plates/new">新規作成</a>
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
  <Modal title={menuPlate.name} onclose={() => void closeMenu()}>
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
        class="btn danger"
        disabled={queueBusy || deleting}
        onclick={() => void remove()}>{deleting ? "削除中…" : "削除"}</button
      >
      {#if menuError}<p role="alert">{menuError}</p>{/if}
    </div>
  </Modal>
{/if}

<style lang="sass">
  .plate-menu
    display: grid
    gap: var(--sp-2)
</style>
