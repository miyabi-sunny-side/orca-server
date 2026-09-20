<script lang="ts">
  import { onMount } from "svelte";
  import {
    request,
    type Printer,
    type Filament,
    type AmsSlot,
    type AmsInventory,
  } from "../lib/api";
  const id = window.location.pathname.split("/")[2];
  let printer = $state<Printer>();
  let filaments = $state<Filament[]>([]);
  let inventory = $state<AmsInventory>();
  let error = $state("");
  let loading = $state(true);
  let busy = $state(false);
  let edit = $state("");
  let revision = $state(0);
  let choice = $state("");
  const controller = new AbortController();
  let refreshing = false;
  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      const [p, f, a] = await Promise.all([
        request<Printer>(`/api/printers/${id}`, { signal: controller.signal }),
        request<Filament[]>("/api/filaments", { signal: controller.signal }),
        request<AmsInventory>(`/api/printers/${id}/ams`, {
          signal: controller.signal,
        }),
      ]);
      if (controller.signal.aborted) return;
      printer = p;
      filaments = f;
      inventory = a;
    } catch (cause) {
      if (!controller.signal.aborted) {
        error = (cause as Error).message;
        inventory = undefined;
      }
    } finally {
      refreshing = false;
      if (!controller.signal.aborted) loading = false;
    }
  }
  onMount(() => {
    void refresh();
    const timer = setInterval(() => {
      if (!busy && !document.hidden) void refresh();
    }, 5000);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  });
  function begin(slot: AmsSlot) {
    edit = slot.id;
    revision = slot.revision;
    choice = slot.filament_id ?? "";
    error = "";
  }
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    try {
      await request(`/api/printers/${id}/ams/${edit}`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ revision, filament_id: choice || null }),
      });
      edit = "";
      await refresh();
    } catch (cause) {
      error = (cause as Error).message;
      await refresh();
    } finally {
      busy = false;
    }
  }
  const fit: Record<string, string> = {
    supported: "既知のノズル条件に適合",
    unsupported: "ノズルの条件に非対応",
    unknown: "ノズル適合は未確認",
  };
</script>

<svelte:head><title>AMSの材料 · OrcaServer</title></svelte:head>
<section class="content" aria-label="AMSの材料">
  <a href="/printers">プリンター一覧へ</a>
  <div class="page-heading">
    <h1>{printer?.name ?? "プリンター"} · AMS</h1>
    <button
      class="btn"
      disabled={busy}
      onclick={() => {
        error = "";
        void refresh();
      }}>状態を更新</button
    >
  </div>
  {#if printer}<p class="caption">
      登録構成: {printer.machine_profile_key} / {printer.nozzle_material ===
      "hardened_steel"
        ? "焼入れ鋼"
        : printer.nozzle_material === "stainless_steel"
          ? "ステンレス"
          : "材質未確認"}　<a href={`/printers/${id}`}>構成を編集</a>
    </p>{/if}
  {#if error}<div class="notice"><p role="alert">{error}</p></div>{/if}
  {#if loading}<p class="state" role="status">読み込んでいます…</p>
  {:else if inventory}
    {#if !inventory.current}<div class="notice">
        <p role="status">
          現在の装填状態は未確認です。保存済みの観測情報を表示しています。接続後の完全な報告を待ってください。
        </p>
      </div>{/if}
    {#if !inventory.slots.length}<p class="state">
        AMSの情報はまだありません。プリンターとAMSの接続を確認してください。
      </p>{/if}
    <ul class="plate-list">
      {#each inventory.slots as slot (slot.id)}
        <li class="ams-card">
          <h2>AMS {slot.ams_id} · スロット {slot.slot_index + 1}</h2>
          <p class="caption">
            {slot.current
              ? slot.reported.present
                ? "装填あり"
                : "空"
              : "現在の装填は未確認"} · 最終観測 {slot.reported.last_seen_at
              ? new Date(slot.reported.last_seen_at * 1000).toLocaleString()
              : "なし"}
          </p>
          <div class="material">
            {#if slot.filament}<strong
                ><span
                  class="swatch"
                  style:background={`#${slot.filament.color}`}
                ></span>{slot.filament.name}</strong
              >
              <p>{slot.filament.vendor} · {slot.filament.material}</p>
            {:else}<strong>材料は未指定</strong>{/if}
            <p class="caption">
              {slot.mapping_source === "automatic"
                ? "ID・色・タグによる自動対応"
                : slot.mapping_source === "manual"
                  ? "手動で指定した対応"
                  : "一致する材料を一意に特定できません"}
            </p>
          </div>
          {#if slot.setting?.resolved}<p>
              設定温度: 初層 {slot.setting.resolved
                .nozzle_temperature_initial_layer ?? "不明"}℃ / 通常 {slot
                .setting.resolved.nozzle_temperature ?? "不明"}℃
            </p>
          {:else if slot.filament}<p>
              この構成の温度設定がありません。<a
                href={`/filaments/${slot.filament.id}`}>材料の設定へ</a
              >
            </p>{/if}
          {#if slot.filament}<p
              class:incompatible={slot.nozzle_fit === "unsupported"}
            >
              {fit[slot.nozzle_fit] ?? fit.unknown}
            </p>{/if}
          {#if slot.filament?.material.endsWith("-GF") || slot.filament?.material.endsWith("-CF")}<p
              class="help"
            >
              繊維入り材料は焼入れ鋼が必要です。0.2 mmは非対応、0.4
              mmは詰まりや摩耗の条件を材料メーカーに確認してください。
            </p>{/if}
          <details>
            <summary>プリンターからの報告</summary>
            <dl>
              <dt>種別・銘柄</dt>
              <dd>
                {slot.reported.material ?? "不明"} / {slot.reported.brand ??
                  "不明"}
              </dd>
              <dt>色</dt>
              <dd>
                {slot.reported.color ? `#${slot.reported.color}` : "不明"}
              </dd>
              <dt>残量</dt>
              <dd>
                {slot.reported.remaining_percent === null
                  ? "不明"
                  : `${slot.reported.remaining_percent}%`}
              </dd>
              <dt>報告温度範囲</dt>
              <dd>
                {slot.reported.temperature_min ?? "不明"}–{slot.reported
                  .temperature_max ?? "不明"}℃（層別の設定値とは別）
              </dd>
              <dt>材料ID</dt>
              <dd>{slot.reported.profile_id ?? "不明"}</dd>
              <dt>タグ</dt>
              <dd>{slot.reported.tag_uid ?? "なし・未取得"}</dd>
              <dt>挿入時の自動識別</dt>
              <dd>
                {slot.detect_on_insert === null
                  ? "不明"
                  : slot.detect_on_insert
                    ? "有効"
                    : "無効"}
              </dd>
              <dt>起動時の自動識別</dt>
              <dd>
                {slot.detect_on_power_up === null
                  ? "不明"
                  : slot.detect_on_power_up
                    ? "有効"
                    : "無効"}
              </dd>
            </dl>
          </details>
          {#if edit === slot.id}
            <form onsubmit={save}>
              <label class="field"
                ><span>対応する材料</span><select
                  bind:value={choice}
                  disabled={busy}
                  ><option value="">対応を解除</option
                  >{#each filaments as f}<option
                      value={f.id}
                      disabled={!slot.current || !slot.reported.present}
                      >{f.name} / {f.material} / #{f.color}</option
                    >{/each}</select
                ></label
              >
              {#if revision !== slot.revision}<p role="alert">
                  観測情報が変わりました。選び直してから保存してください。
                </p>
                <button class="btn" type="button" onclick={() => begin(slot)}
                  >選び直す</button
                >{/if}
              <div class="actions">
                <button
                  class="btn primary"
                  disabled={busy ||
                    revision !== slot.revision ||
                    (!!choice && (!slot.current || !slot.reported.present))}
                  >対応を保存</button
                ><button
                  type="button"
                  class="btn"
                  disabled={busy}
                  onclick={() => (edit = "")}>キャンセル</button
                >
              </div>
            </form>
          {:else}<button class="btn" disabled={busy} onclick={() => begin(slot)}
              >材料を指定・解除</button
            >{/if}
        </li>
      {/each}
    </ul>
  {/if}
  <p class="help">
    タグのない材料を交換し、報告値が同じ場合は自動で見分けられません。交換後に材料を指定し直してください。
  </p>
  <a href="/filaments">材料台帳を開く</a>
</section>

<style lang="sass">
  .page-heading
    margin-top: var(--sp-3)
  .ams-card
    padding: var(--sp-3)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)
    overflow-wrap: anywhere
  p
    margin: var(--sp-2) 0
  .material
    padding: var(--sp-2) 0
  .swatch
    display: inline-block
    width: 1em
    height: 1em
    margin-right: var(--sp-2)
    border: 1px solid var(--c-muted)
    border-radius: var(--radius-full)
  details
    margin: var(--sp-3) 0
  summary
    cursor: pointer
  dl
    font-size: var(--fs-sm)
  dt
    color: var(--c-muted)
  dd
    margin: 0 0 var(--sp-2)
  .incompatible
    color: var(--c-danger)
</style>
