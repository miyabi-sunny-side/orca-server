<script lang="ts">
  import { onMount } from "svelte";
  import {
    request,
    ApiError,
    type FilamentProduct,
    type Machine,
    type FilamentProfile,
  } from "../lib/api";
  const parts = window.location.pathname.split("/");
  const id = parts[2] === "new" ? "" : parts[2];
  const productMode = parts[2] === "new" || parts[3] === "edit";
  const colorMode = parts[3] === "colors";
  const settingMode = parts[3] === "settings";
  const childId = parts[4] === "new" ? "" : parts[4];
  const sid = childId;
  const formMode = productMode || colorMode || settingMode;
  const params = new URLSearchParams(location.search);
  const returnPrinter = params.get("return_queue");
  const back = returnPrinter
    ? `/?printer_id=${encodeURIComponent(returnPrinter)}#job-${encodeURIComponent(params.get("return_job") ?? "")}`
    : id
      ? `/filaments/${id}`
      : "/filaments";
  const api = id ? `/api/filament-products/${id}` : "/api/filament-products";
  let items = $state<FilamentProduct[]>([]);
  let product = $state<FilamentProduct>();
  let settings = $state<FilamentProduct["settings"]>([]);
  let machines = $state<Machine[]>([]);
  let profiles = $state<FilamentProfile[]>([]);
  let data = $state({
    name: "",
    vendor: "",
    material: "PLA",
    bambu_filament_id: "",
  });
  let color = $state({ name: "", color: "FFFFFFFF" });
  let loading = $state(true),
    loaded = $state(false),
    busy = $state(false),
    error = $state(""),
    profileError = $state("");
  let machine = $state(""),
    base = $state(""),
    adoptId = $state("");
  let first = $state<number>(),
    normal = $state<number>(),
    bedFirst = $state<number>(),
    bedNormal = $state<number>();
  const selected = $derived(profiles.find((p) => p.key === base));
  const presets = [
    { name: "黒", color: "000000FF" },
    { name: "白", color: "FFFFFFFF" },
    { name: "黄", color: "FFFF00FF" },
    { name: "赤", color: "FF0000FF" },
    { name: "青", color: "0000FFFF" },
    { name: "透明", color: "FFFFFF00" },
  ];
  const controller = new AbortController();
  let sequence = 0;
  async function openColor() {
    const legacy = await request<{
      product_id: string;
      settings: FilamentProduct["settings"];
    }>(`/api/filaments/${id}`, { signal: controller.signal });
    const machine = params.get("machine");
    const setting = legacy.settings.find(
      (s) => s.machine_profile_key === machine,
    );
    window.location.replace(
      machine
        ? `/filaments/${legacy.product_id}/settings/${setting?.id ?? "new"}?${params.toString()}#bed-temperature`
        : `/filaments/${legacy.product_id}/colors/${id}`,
    );
  }
  async function loadProfiles() {
    const ticket = ++sequence;
    profiles = [];
    profileError = "";
    if (!machine) return;
    try {
      const result = await request<FilamentProfile[]>(
        `${api}/profiles?machine=${encodeURIComponent(machine)}`,
        { signal: controller.signal },
      );
      if (ticket === sequence && !controller.signal.aborted) profiles = result;
    } catch (cause) {
      if (ticket === sequence && !controller.signal.aborted)
        profileError = (cause as Error).message;
    }
  }
  async function load() {
    loading = true;
    loaded = false;
    error = "";
    try {
      if (id) {
        if (!parts[3] && params.has("machine")) {
          await openColor();
          return;
        }
        try {
          product = await request<FilamentProduct>(api, {
            signal: controller.signal,
          });
        } catch (cause) {
          if (cause instanceof ApiError && cause.status === 404 && !parts[3]) {
            await openColor();
            return;
          }
          throw cause;
        }
        data = {
          name: product.name,
          vendor: product.vendor,
          material: product.material,
          bambu_filament_id: product.bambu_filament_id ?? "",
        };
        settings = product.settings;
        if (colorMode && childId) {
          const c = product.colors.find((c) => c.id === childId);
          if (!c)
            throw new Error("色が見つかりません。製品から開き直してください。");
          color = { name: c.name, color: c.color };
        }
      }
      if (!formMode)
        items = await request<FilamentProduct[]>("/api/filament-products", {
          signal: controller.signal,
        });
      if (settingMode) {
        machines = await request<Machine[]>("/api/printers/profiles", {
          signal: controller.signal,
        });
        if (!sid) machine = params.get("machine") ?? "";
        if (sid) {
          const s = settings.find((s) => s.id === sid);
          if (!s)
            throw new Error(
              "設定が見つかりません。製品から開き直してください。",
            );
          machine = s.machine_profile_key;
          base = s.base_profile_key;
          first = s.overrides_json.nozzle_temperature_initial_layer;
          normal = s.overrides_json.nozzle_temperature;
          bedFirst = s.overrides_json.bed_temperature_initial_layer;
          bedNormal = s.overrides_json.bed_temperature;
        }
        await loadProfiles();
      }
      loaded = true;
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
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    busy = true;
    error = "";
    try {
      if (settingMode) {
        await request(`${api}/settings${sid ? `/${sid}` : ""}`, {
          method: sid ? "PUT" : "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            machine_profile_key: machine,
            base_profile_key: base,
            overrides_json: {
              nozzle_temperature_initial_layer: first,
              nozzle_temperature: normal,
              bed_temperature_initial_layer: bedFirst,
              bed_temperature: bedNormal,
            },
          }),
        });
      } else if (colorMode) {
        await request(`${api}/colors${childId ? `/${childId}` : ""}`, {
          method: childId ? "PUT" : "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(color),
        });
      } else {
        const saved = await request<FilamentProduct>(api, {
          method: id ? "PUT" : "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            ...data,
            bambu_filament_id: data.bambu_filament_id || null,
          }),
        });
        window.location.assign(`/filaments/${saved.id}`);
        return;
      }
      window.location.assign(back);
    } catch (cause) {
      error = (cause as Error).message;
      busy = false;
    }
  }
  async function remove() {
    if (
      busy ||
      !window.confirm(
        settingMode
          ? "全色に共通の機種用設定を削除しますか？"
          : colorMode
            ? `「${color.name}」を削除しますか？`
            : `「${data.name}」と全ての色・設定を削除しますか？`,
      )
    )
      return;
    busy = true;
    error = "";
    try {
      await request(
        `${api}${settingMode ? `/settings/${sid}` : colorMode ? `/colors/${childId}` : ""}`,
        { method: "DELETE" },
      );
      window.location.assign(formMode ? back : "/filaments");
    } catch (cause) {
      error = (cause as Error).message;
      busy = false;
    }
  }
  async function adopt(event: SubmitEvent) {
    event.preventDefault();
    if (busy || !adoptId) return;
    busy = true;
    error = "";
    try {
      await request(`${api}/adopt`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ filament_id: adoptId }),
      });
      adoptId = "";
      await load();
    } catch (cause) {
      error = (cause as Error).message;
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head
  ><title>{id ? data.name : "フィラメント"} · OrcaServer</title></svelte:head
>
<section class="content" aria-label="フィラメント管理">
  {#if id || productMode}<a href={formMode ? back : "/filaments"}
      >{returnPrinter
        ? "キューへ戻る"
        : formMode && id
          ? "製品へ戻る"
          : "フィラメント一覧へ"}</a
    >{/if}
  <div class="page-heading">
    <h1>
      {settingMode
        ? `${data.name} · 共通設定`
        : colorMode
          ? `${data.name} · ${childId ? "色を編集" : "色を追加"}`
          : productMode
            ? id
              ? "製品を編集"
              : "製品を追加"
            : id
              ? data.name
              : "フィラメント"}
    </h1>
    {#if !id && !formMode}<a class="btn primary" href="/filaments/new"
        >製品を追加</a
      >{/if}
  </div>
  {#if error}<div class="notice"><p role="alert">{error}</p></div>{/if}
  {#if loading}<p class="state" role="status">読み込んでいます…</p>
  {:else if !loaded}<button class="btn" onclick={() => void load()}
      >読み直す</button
    >
  {:else if formMode}
    <form onsubmit={save}>
      <fieldset disabled={busy}>
        {#if settingMode}
          <label class="field"
            ><span>機種・ノズル径</span><select
              required
              bind:value={machine}
              onchange={() => {
                base = "";
                void loadProfiles();
              }}
              ><option value="" disabled>構成を選択</option
              >{#each machines as m}<option value={m.key}
                  >{m.model} / {m.nozzle_diameter} mm</option
                >{/each}</select
            ></label
          >
          {#if profileError}<div class="notice">
              <p role="alert">{profileError}</p>
              <button
                type="button"
                class="btn"
                onclick={() => void loadProfiles()}>再読込み</button
              >
            </div>{/if}
          <label class="field"
            ><span>基本のフィラメントプロファイル</span><select
              required
              bind:value={base}
              disabled={!profiles.length}
              ><option value="" disabled>対応する設定を選択</option
              >{#if base && !selected}<option value={base}
                  >要確認: {base}</option
                >{/if}{#each profiles as p}<option value={p.key}>{p.key}</option
                >{/each}</select
            ></label
          >
          {#if machine && !profiles.length && !profileError}<p class="help">
              対応するプロファイルがありません。別のノズル径の設定は流用できません。
            </p>{/if}
          <h2>ノズル温度の調整</h2>
          <p class="help">
            キューでこの材料を指定すると、印刷準備の開始時に基本プロファイルと温度設定を適用します。
          </p>
          <p class="help">空欄は基本プロファイルの温度を使用します。</p>
          <label class="field"
            ><span>初層（℃）</span><input
              type="number"
              min="120"
              max="350"
              step="1"
              bind:value={first}
              placeholder={selected?.resolved
                .nozzle_temperature_initial_layer ?? "未選択"}
            /></label
          >
          <label class="field"
            ><span>通常層（℃）</span><input
              type="number"
              min="120"
              max="350"
              step="1"
              bind:value={normal}
              placeholder={selected?.resolved.nozzle_temperature ?? "未選択"}
            /></label
          >
          {#if selected}<p class="caption">
              基本値: 初層 {selected.resolved
                .nozzle_temperature_initial_layer ?? "不明"}℃ / 通常 {selected
                .resolved.nozzle_temperature ?? "不明"}℃
            </p>{/if}
          <h2 id="bed-temperature">ベッド温度の調整</h2>
          <p class="help">
            空欄は選択したビルドプレートの基本温度を使用します。指定した温度はプレートの種類にかかわらず適用します。
          </p>
          <label class="field"
            ><span>ベッド初層（℃）</span><input
              type="number"
              min="0"
              max="120"
              step="1"
              bind:value={bedFirst}
              placeholder="基本プロファイル"
            /></label
          >
          <label class="field"
            ><span>ベッド通常層（℃）</span><input
              type="number"
              min="0"
              max="120"
              step="1"
              bind:value={bedNormal}
              placeholder="基本プロファイル"
            /></label
          >
          <p class="help">
            材料メーカーの推奨温度とプリンターの仕様を確認してください。同じ構成のプリンターでこの設定を共有します。
          </p>
        {:else if colorMode}
          <label class="field"
            ><span>基本色</span><select
              value={color.name &&
              presets.some((p) => p.color === color.color.toUpperCase())
                ? color.color.toUpperCase()
                : ""}
              onchange={(event) => {
                const preset = presets.find(
                  (p) => p.color === event.currentTarget.value,
                );
                if (preset) color = { ...preset };
              }}
              ><option value="">カスタム</option
              >{#each presets as preset}<option value={preset.color}
                  >{preset.name}</option
                >{/each}</select
            ></label
          >
          <label class="field"
            ><span>色名</span><input
              required
              maxlength="160"
              bind:value={color.name}
              placeholder="例: アイボリーホワイト"
            /></label
          >
          <label class="field"
            ><span>色見本</span><input
              type="color"
              value={/^[0-9a-fA-F]{8}$/.test(color.color)
                ? `#${color.color.slice(0, 6)}`
                : "#ffffff"}
              oninput={(event) =>
                (color.color =
                  event.currentTarget.value.slice(1).toUpperCase() +
                  (color.color.slice(6) || "FF"))}
            /></label
          >
          <details>
            <summary>正確な色を編集</summary>
            <label class="field"
              ><span>色（RGBA・8桁）</span><input
                required
                pattern={"[0-9a-fA-F]{8}"}
                maxlength="8"
                bind:value={color.color}
              /></label
            >
            <p class="help">
              000000はRGB
              0/0/0、161616は22/22/22の異なる色です。末尾2桁は透明度で、FFは不透明です。AMSとの照合には正確な値を使います。
            </p>
          </details>
        {:else}
          <label class="field"
            ><span>製品名</span><input
              required
              maxlength="160"
              bind:value={data.name}
              placeholder="例: Bambu PLA Matte"
            /></label
          >
          <label class="field"
            ><span>メーカー</span><input
              required
              maxlength="160"
              bind:value={data.vendor}
            /></label
          >
          <label class="field"
            ><span>材料種別</span><input
              required
              maxlength="160"
              list="materials"
              bind:value={data.material}
            /></label
          >
          <datalist id="materials"
            >{#each ["PLA", "PETG", "PETG-GF", "PETG-CF", "ABS", "ASA", "TPU", "PA", "PA-CF"] as m}<option
                value={m}
              ></option>{/each}</datalist
          >
          <details>
            <summary>自動識別の詳細</summary><label class="field"
              ><span>Bambu材料ID（純正品・任意）</span><input
                maxlength="64"
                bind:value={data.bambu_filament_id}
                placeholder="例: GFA01"
              /></label
            >
            <p class="help">
              ID・正確な色・材料種別・タグが一致した候補を使います。サードパーティ製は空欄にしてAMSで対応づけます。
            </p>
          </details>
        {/if}
      </fieldset>
      <div class="actions">
        <button
          class="btn primary"
          disabled={busy || (settingMode && !selected)}
          >{busy ? "保存しています…" : "保存"}</button
        ><a class="btn" href={back}>戻る</a>
      </div>
    </form>
    {#if childId && (colorMode || settingMode)}<div class="delete-area">
        <button class="btn" disabled={busy} onclick={() => void remove()}
          >{settingMode ? "この設定を削除" : "この色を削除"}</button
        >
      </div>{/if}
  {:else if !id}
    {#if !items.length}<p class="state">
        製品を登録して、使用する色を追加してください。
      </p>{/if}
    <ul class="plate-list">
      {#each items as p (p.id)}<li>
          <a class="plate-row" href={`/filaments/${p.id}`}
            ><strong>{p.name}</strong><span class="caption"
              >{p.vendor} · {p.material}</span
            ><span class="colors"
              >{#each p.colors as c (c.id)}<span
                  ><span class="swatch" style:background={`#${c.color}`}
                  ></span>{c.name}</span
                >{/each}</span
            ></a
          >
        </li>{/each}
    </ul>
    <a href="/printers">プリンターとAMSを確認</a>
  {:else if product}
    <p class="caption">{product.vendor} · {product.material}</p>
    <a href={`/filaments/${id}/edit`}>共通情報を編集</a>
    <div class="page-heading">
      <h2>色</h2>
      <a class="btn primary" href={`/filaments/${id}/colors/new`}>色を追加</a>
    </div>
    {#if !product.colors.length}<p class="help">
        色を追加するとAMSに対応づけられます。
      </p>{/if}
    <ul class="plate-list">
      {#each product.colors as c (c.id)}<li>
          <a class="plate-row" href={`/filaments/${id}/colors/${c.id}`}
            ><strong
              ><span class="swatch" style:background={`#${c.color}`}
              ></span>{c.name}</strong
            ></a
          >
        </li>{/each}
    </ul>
    <section class="settings">
      <div class="page-heading">
        <h2>全色に共通の機種別設定</h2>
        <a class="btn" href={`/filaments/${id}/settings/new`}>設定を追加</a>
      </div>
      {#if !settings.length}<p class="help">
          使う機種・ノズル径ごとに基本プロファイルを選択してください。
        </p>{/if}
      <ul class="plate-list">
        {#each settings as s (s.id)}<li>
            <a class="plate-row" href={`/filaments/${id}/settings/${s.id}`}
              ><strong>{s.machine_profile_key}</strong><span class="caption"
                >{s.base_profile_key}</span
              ><span
                >初層 {s.resolved?.nozzle_temperature_initial_layer ?? "不明"}℃
                / 通常 {s.resolved?.nozzle_temperature ?? "不明"}℃</span
              >{#if s.overrides_json.bed_temperature_initial_layer !== undefined || s.overrides_json.bed_temperature !== undefined}<span
                  class="caption"
                  >ベッド: 初層 {s.overrides_json
                    .bed_temperature_initial_layer === undefined
                    ? "基本値"
                    : `${s.overrides_json.bed_temperature_initial_layer}℃`} / 通常
                  {s.overrides_json.bed_temperature === undefined
                    ? "基本値"
                    : `${s.overrides_json.bed_temperature}℃`}</span
                >{/if}{#if s.error}<span role="alert"
                  >基本プロファイルを選び直してください。</span
                >{/if}</a
            >
          </li>{/each}
      </ul>
    </section>
    <details class="settings">
      <summary>既存の色をまとめる</summary>
      <p class="help">
        メーカー・材料・Bambu
        ID・全機種の設定が同じ登録を、この製品の色としてまとめます。材料IDとAMSの対応は保持します。
      </p>
      <form onsubmit={adopt}>
        <fieldset disabled={busy}>
          <label class="field"
            ><span>既存の色</span><select required bind:value={adoptId}
              ><option value="" disabled>色を選択</option
              >{#each items.filter((p) => p.id !== id) as p}{#each p.colors as c}<option
                    value={c.id}>{p.name} / {c.name}</option
                  >{/each}{/each}</select
            ></label
          ><button class="btn" disabled={!adoptId || busy}
            >この製品にまとめる</button
          >
        </fieldset>
      </form>
    </details>
    <details class="delete-area">
      <summary>製品の管理</summary>
      <p class="help">AMSや印刷ジョブが参照する色・設定は削除できません。</p>
      <button class="btn" disabled={busy} onclick={() => void remove()}
        >製品を削除</button
      >
    </details>
  {/if}
</section>

<style lang="sass">
  .page-heading
    margin-top: var(--sp-3)
  h1, .settings, .colors
    overflow-wrap: anywhere
  fieldset
    margin: 0
    padding: 0
    border: 0
    min-width: 0
  .settings, .delete-area
    margin-top: var(--sp-5)
    padding-top: var(--sp-4)
    border-top: 1px solid var(--c-border)
  .colors
    display: flex
    flex-wrap: wrap
    gap: var(--sp-2)
  .swatch
    display: inline-block
    width: 1em
    height: 1em
    margin-right: var(--sp-2)
    border: 1px solid var(--c-muted)
    border-radius: var(--radius-full)
</style>
