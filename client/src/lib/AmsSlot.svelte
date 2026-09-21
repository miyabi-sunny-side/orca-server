<script lang="ts">
  import { tick } from "svelte";
  import { request, type AmsSlot, type Filament } from "./api";
  let {
    slot,
    printerId,
    slots,
    busy,
    mutate,
  }: {
    slot: AmsSlot;
    printerId: string;
    slots: AmsSlot[];
    busy: boolean;
    mutate: (path: string, data: unknown) => Promise<boolean>;
  } = $props();
  let expanded = $state(false),
    searching = $state(false),
    query = $state("");
  let revision = $state(0),
    options = $state<Filament[]>([]),
    loading = $state(false),
    error = $state("");
  let input = $state<HTMLInputElement>(),
    list = $state<HTMLUListElement>(),
    selector = $state<HTMLButtonElement>();
  const name = $derived(`AMS ${slot.ams_id} スロット ${slot.slot_index + 1}`);
  const group = $derived(slot.priority_group ?? []);
  const stale = $derived(revision !== slot.revision);
  const root = $derived(`/api/printers/${printerId}/ams`);
  const peers = $derived(
    slot.backup_peers?.map((n) =>
      slots.find((s) => s.ams_id * 4 + s.slot_index === n),
    ) ?? null,
  );
  $effect(() => {
    if (!searching) return;
    const q = query,
      controller = new AbortController();
    loading = true;
    error = "";
    const timer = setTimeout(async () => {
      try {
        const value = await request<Filament[]>(
          `/api/filaments?q=${encodeURIComponent(q)}`,
          { signal: controller.signal },
        );
        if (!controller.signal.aborted) options = value;
      } catch (cause) {
        if (!controller.signal.aborted) error = (cause as Error).message;
      } finally {
        if (!controller.signal.aborted) loading = false;
      }
    }, 150);
    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  });
  async function begin() {
    revision = slot.revision;
    query = "";
    searching = true;
    await tick();
    input?.focus();
  }
  async function close() {
    searching = false;
    await tick();
    selector?.focus();
  }
  async function choose(filament: string | null) {
    if (await mutate(`${root}/${slot.id}`, { revision, filament_id: filament }))
      await close();
  }
  async function prioritize(index: number) {
    const order = [...group],
      old = order.findIndex((s) => s.id === slot.id);
    if (old < 0 || old === index) return;
    order.splice(index, 0, ...order.splice(old, 1));
    await mutate(`${root}/priority`, { filament_id: slot.filament_id, order });
  }
  function move(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      void close();
      return;
    }
    if (!["ArrowDown", "ArrowUp"].includes(event.key)) return;
    event.preventDefault();
    const buttons = Array.from(
      list?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [],
    );
    const index = buttons.indexOf(event.target as HTMLButtonElement);
    const next = index + (event.key === "ArrowDown" ? 1 : -1);
    if (next < 0) input?.focus();
    else buttons[next]?.focus();
  }
</script>

<li class="ams-slot">
  <div class="slot-heading">
    <span class="caption" title={name}
      >{slot.ams_id} · {slot.slot_index + 1}</span
    >
    <button
      class="selector"
      bind:this={selector}
      aria-label={`${name}の材料を選択`}
      aria-expanded={searching}
      disabled={busy}
      onclick={begin}
    >
      {#if slot.filament}<span
          class="swatch"
          style:background={`#${slot.filament.color}`}
        ></span>{/if}
      <span
        >{slot.filament?.name ??
          (slot.reported.present === false && slot.current
            ? "空"
            : "材料は未指定")}</span
      ><span aria-hidden="true">▾</span>
    </button>
    <button
      class="btn toggle"
      aria-label={`${name}の詳細`}
      aria-expanded={expanded}
      onclick={() => (expanded = !expanded)}>{expanded ? "−" : "+"}</button
    >
  </div>
  {#if searching}
    <div class="selection">
      <div class="search-row">
        <input
          type="search"
          aria-label="材料を検索"
          placeholder="製品・メーカー・材質・色"
          bind:value={query}
          bind:this={input}
          onkeydown={move}
        />
        <button class="btn" disabled={busy} onclick={close}>キャンセル</button>
      </div>
      {#if stale}<p role="alert">
          観測情報が変わりました。選び直してから指定してください。
        </p>
        <button
          class="btn"
          disabled={busy}
          onclick={() => (revision = slot.revision)}>選び直す</button
        >{/if}
      {#if loading}<p class="caption" role="status">検索中…</p>{/if}
      {#if error}<p role="alert">{error}</p>{/if}
      <ul class="choices" bind:this={list}>
        <li>
          <button
            disabled={busy || stale}
            onclick={() => choose(null)}
            onkeydown={move}>指定を解除</button
          >
        </li>
        {#if !loading && !error}{#each options as f (f.id)}<li>
              <button
                disabled={busy ||
                  stale ||
                  !slot.current ||
                  !slot.reported.present}
                onclick={() => choose(f.id)}
                onkeydown={move}
              >
                <span class="swatch" style:background={`#${f.color}`}
                ></span><span
                  >{f.name}<small>{f.vendor} · {f.material}</small></span
                >
              </button>
            </li>{/each}{/if}
      </ul>
      {#if !loading && !error && !options.length}<p class="caption">
          一致する材料がありません。<a href="/filaments">材料を登録</a>
        </p>{/if}
    </div>
  {/if}
  {#if expanded}
    <div class="slot-details">
      <p class="caption">
        {slot.current
          ? slot.reported.present
            ? "装填あり"
            : "空"
          : "現在の装填は未確認"} · {slot.mapping_source === "automatic"
          ? "ID・色・タグによる自動対応"
          : slot.mapping_source === "manual"
            ? "手動で指定した対応"
            : "材料は未指定"}
      </p>
      {#if group.length > 1}<label class="field"
          ><span>印刷開始時の使用順</span><select
            value={group.findIndex((s) => s.id === slot.id)}
            disabled={busy || !slot.current}
            onchange={(e) => prioritize(Number(e.currentTarget.value))}
            >{#each group as _, i}<option value={i}>{i + 1}番目</option
              >{/each}</select
          ></label
        >{/if}
      <p class="help">
        同じ製品・色は先に装填した方から使います。未観測のタグなし交換は見分けられないため、必要に応じて使用順を変更してください。
      </p>
      {#if slot.setting?.resolved}<p>
          設定温度: 初層 {slot.setting.resolved
            .nozzle_temperature_initial_layer ?? "不明"}℃ / 通常 {slot.setting
            .resolved.nozzle_temperature ?? "不明"}℃
        </p>
      {:else if slot.filament}<p>
          この構成の温度設定がありません。<a
            href={`/filaments/${slot.filament.id}`}>材料の設定へ</a
          >
        </p>{/if}
      <dl>
        <dt>機器が認識する補充先</dt>
        <dd>
          {peers === null
            ? "未報告"
            : peers.length
              ? peers
                  .map((s) =>
                    s
                      ? `AMS ${s.ams_id} · ${s.slot_index + 1} ${s.filament?.name ?? "材料未指定"}`
                      : "未観測のスロット",
                  )
                  .join("、")
              : "なし"}
        </dd>
        {#if peers?.some((p) => !p || !group.some((s) => s.id === p.id))}<dt>
            材料の確認
          </dt>
          <dd>
            機器の補充先に、登録上は同一と確認できない材料があります。プリンター側の材料設定を確認してください。
          </dd>{/if}
        <dt>種別・銘柄</dt>
        <dd>
          {slot.reported.material ?? "不明"} / {slot.reported.brand ?? "不明"}
        </dd>
        <dt>色</dt>
        <dd>{slot.reported.color ? `#${slot.reported.color}` : "不明"}</dd>
        <dt>残量</dt>
        <dd>
          {slot.reported.remaining_percent === null
            ? "不明"
            : `${slot.reported.remaining_percent}%`}
        </dd>
        <dt>報告温度範囲</dt>
        <dd>
          {slot.reported.temperature_min ?? "不明"}–{slot.reported
            .temperature_max ?? "不明"}℃
        </dd>
        <dt>材料ID・タグ</dt>
        <dd>
          {slot.reported.profile_id ?? "不明"} / {slot.reported.tag_uid ??
            "なし・未取得"}
        </dd>
        <dt>自動識別</dt>
        <dd>
          挿入時 {slot.detect_on_insert === null
            ? "不明"
            : slot.detect_on_insert
              ? "有効"
              : "無効"} / 起動時 {slot.detect_on_power_up === null
            ? "不明"
            : slot.detect_on_power_up
              ? "有効"
              : "無効"}
        </dd>
        <dt>最終観測</dt>
        <dd>
          {slot.reported.last_seen_at
            ? new Date(slot.reported.last_seen_at * 1000).toLocaleString()
            : "なし"}
        </dd>
      </dl>
    </div>
  {/if}
</li>

<style lang="sass">
  .ams-slot
    border-bottom: 1px solid var(--c-border)
    padding: var(--sp-2) 0
    overflow-wrap: anywhere
  .slot-heading
    display: grid
    grid-template-columns: auto minmax(0, 1fr) auto
    align-items: center
    gap: var(--sp-2)
  .selector
    display: flex
    align-items: center
    gap: var(--sp-2)
    text-align: left
    min-width: 0
    padding: var(--sp-2)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
    background: var(--c-surface)
    color: var(--c-on-surface)
    font: inherit
    cursor: pointer
    > span:nth-last-child(2)
      flex: 1
    > span:last-child
      margin-left: auto
  .toggle
    min-width: 36px
  .swatch
    width: 16px
    height: 16px
    flex: 0 0 16px
    border: 1px solid var(--c-muted)
    border-radius: 50%
  .selection, .slot-details
    padding: var(--sp-3) 0
  .search-row
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)
    input
      flex: 1
      min-width: 120px
      width: 100%
  .choices
    list-style: none
    margin: var(--sp-2) 0 0
    padding: 0
    button
      display: flex
      align-items: center
      gap: var(--sp-2)
      width: 100%
      padding: var(--sp-2)
      color: var(--c-on-surface)
      background: var(--c-surface)
      border: 0
      border-bottom: 1px solid var(--c-border)
      text-align: left
      font: inherit
      cursor: pointer
      &:hover:not(:disabled)
        background: var(--c-hover-1)
      &:disabled
        opacity: .5
    small
      display: block
      color: var(--c-muted)
  p
    margin: var(--sp-2) 0
  dl
    margin: var(--sp-3) 0
  dt
    color: var(--c-muted)
    font-size: .875em
  dd
    margin: 0 0 var(--sp-2)
</style>
