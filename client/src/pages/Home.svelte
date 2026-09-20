<script lang="ts">
  import { request, type Plate } from "../lib/api";
  let query = $state("");
  let revision = $state(0);
  let plates = $state<Plate[]>([]);
  let phase = $state<"loading" | "error" | "success">("loading");
  let error = $state("");
  let search = $state<HTMLInputElement>();
  let list = $state<HTMLUListElement>();

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
          <a class="plate-row" href={`/plates/${plate.id}`} onkeydown={move}>
            <strong>{plate.name}</strong>
            <span class="caption"
              >{plate.models.length}モデル · {plate.print
                ? "配置済み"
                : "スライス未完了"}</span
            >
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</section>
