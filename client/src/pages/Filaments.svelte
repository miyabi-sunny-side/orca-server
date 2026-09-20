<script lang="ts">
  import { onMount } from "svelte";
  import {
    request,
    type Filament,
    type FilamentSetting,
    type Machine,
    type FilamentProfile,
  } from "../lib/api";
  const parts = window.location.pathname.split("/");
  const editing = !!parts[2];
  const id = parts[2] === "new" ? "" : parts[2];
  const settingMode = parts[3] === "settings";
  const sid = parts[4] === "new" ? "" : parts[4];
  let items = $state<Filament[]>([]);
  let settings = $state<FilamentSetting[]>([]);
  let machines = $state<Machine[]>([]);
  let profiles = $state<FilamentProfile[]>([]);
  let loading = $state(true);
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state("");
  let profileError = $state("");
  let data = $state({
    name: "",
    vendor: "",
    material: "PLA",
    color: "FFFFFFFF",
    bambu_filament_id: "",
  });
  let machine = $state("");
  let base = $state("");
  let first = $state<number>();
  let normal = $state<number>();
  const selected = $derived(profiles.find((p) => p.key === base));
  const controller = new AbortController();
  let sequence = 0;
  async function loadProfiles() {
    const ticket = ++sequence;
    profiles = [];
    profileError = "";
    if (!machine) return;
    try {
      const result = await request<FilamentProfile[]>(
        `/api/filaments/${id}/profiles?machine=${encodeURIComponent(machine)}`,
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
        const value = await request<{
          filament: Filament;
          settings: FilamentSetting[];
        }>(`/api/filaments/${id}`, { signal: controller.signal });
        const f = value.filament;
        data = {
          name: f.name,
          vendor: f.vendor,
          material: f.material,
          color: f.color,
          bambu_filament_id: f.bambu_filament_id ?? "",
        };
        settings = value.settings;
      } else if (!editing)
        items = await request<Filament[]>("/api/filaments", {
          signal: controller.signal,
        });
      if (settingMode) {
        machines = await request<Machine[]>("/api/printers/profiles", {
          signal: controller.signal,
        });
        if (sid) {
          const s = settings.find((s) => s.id === sid);
          if (!s)
            throw new Error(
              "設定が見つかりません。材料の画面から開き直してください。",
            );
          machine = s.machine_profile_key;
          base = s.base_profile_key;
          first = s.overrides_json.nozzle_temperature_initial_layer;
          normal = s.overrides_json.nozzle_temperature;
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
        await request(`/api/filaments/${id}/settings${sid ? `/${sid}` : ""}`, {
          method: sid ? "PUT" : "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            machine_profile_key: machine,
            base_profile_key: base,
            overrides_json: {
              nozzle_temperature_initial_layer: first,
              nozzle_temperature: normal,
            },
          }),
        });
        window.location.assign(`/filaments/${id}`);
      } else {
        const f = await request<Filament>(
          `/api/filaments${id ? `/${id}` : ""}`,
          {
            method: id ? "PUT" : "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              ...data,
              bambu_filament_id: data.bambu_filament_id || null,
            }),
          },
        );
        window.location.assign(`/filaments/${f.id}`);
      }
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
          ? "この機種用設定を削除しますか？"
          : `「${data.name}」と機種用設定を削除しますか？`,
      )
    )
      return;
    busy = true;
    error = "";
    try {
      await request(
        `/api/filaments/${id}${settingMode ? `/settings/${sid}` : ""}`,
        { method: "DELETE" },
      );
      window.location.assign(settingMode ? `/filaments/${id}` : "/filaments");
    } catch (cause) {
      error = (cause as Error).message;
      busy = false;
    }
  }
</script>

<svelte:head
  ><title
    >{settingMode
      ? "機種別の材料設定"
      : editing
        ? "材料を編集"
        : "フィラメント"} · OrcaServer</title
  ></svelte:head
>
<section class="content" aria-label="フィラメント管理">
  {#if editing}<a href={settingMode ? `/filaments/${id}` : "/filaments"}
      >{settingMode ? "材料へ戻る" : "材料一覧へ"}</a
    >{/if}
  <div class="page-heading">
    <h1>
      {settingMode
        ? `${data.name} · 機種別設定`
        : editing
          ? id
            ? "材料を編集"
            : "材料を追加"
          : "フィラメント"}
    </h1>
    {#if !editing}<a class="btn primary" href="/filaments/new">追加</a>{/if}
  </div>
  {#if error}<div class="notice"><p role="alert">{error}</p></div>{/if}
  {#if loading}<p class="state" role="status">読み込んでいます…</p>
  {:else if !loaded}<button class="btn" onclick={() => void load()}
      >読み直す</button
    >
  {:else if !editing}
    {#if items.length === 0}<p class="state">
        使用するフィラメントを銘柄と色ごとに登録してください。AMSのスロットと対応づけられます。
      </p>{/if}
    <ul class="plate-list">
      {#each items as f (f.id)}<li>
          <a class="plate-row" href={`/filaments/${f.id}`}
            ><strong
              ><span class="swatch" style:background={`#${f.color}`}
              ></span>{f.name}</strong
            ><span class="caption">{f.vendor} · {f.material} · #{f.color}</span
            ></a
          >
        </li>{/each}
    </ul>
    <a href="/printers">プリンターとAMSを確認</a>
  {:else}
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
            材料管理用の設定です。スライスにはプレート作成画面のプロファイルが使われます。
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
          <p class="help">
            材料メーカーの推奨温度とプリンターの仕様を確認してください。同じ構成のプリンターでこの設定を共有します。
          </p>
        {:else}
          <label class="field"
            ><span>材料名</span><input
              required
              maxlength="160"
              bind:value={data.name}
              placeholder="例: PLA Matte 黒"
            /></label
          >
          <label class="field"
            ><span>メーカー・銘柄</span><input
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
          <label class="field"
            ><span>色（RGBA・8桁）</span><input
              required
              pattern={"[0-9a-fA-F]{8}"}
              maxlength="8"
              bind:value={data.color}
              aria-describedby="color-help"
            /></label
          >
          <p id="color-help" class="help">
            黒: 000000FF / 白: FFFFFFFF。末尾2桁は透明度です。
          </p>
          <label class="field"
            ><span>Bambu材料ID（純正品・任意）</span><input
              maxlength="64"
              bind:value={data.bambu_filament_id}
              placeholder="例: GFA01"
            /></label
          >
          <p class="help">
            ID・色・材料種別とタグ情報が一致すると自動で対応づけます。サードパーティ製は空欄にして、AMS画面で指定してください。
          </p>
        {/if}
      </fieldset>
      <div class="actions">
        <button
          class="btn primary"
          disabled={busy || (settingMode && !selected)}
          >{busy ? "保存しています…" : "保存"}</button
        ><a class="btn" href={settingMode ? `/filaments/${id}` : "/filaments"}
          >戻る</a
        >
      </div>
    </form>
    {#if id && !settingMode}
      <section class="settings">
        <div class="page-heading">
          <h2>機種別の材料設定</h2>
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
                  >初層 {s.resolved?.nozzle_temperature_initial_layer ??
                    "不明"}℃ / 通常 {s.resolved?.nozzle_temperature ??
                    "不明"}℃</span
                >{#if s.error}<span role="alert"
                    >基本プロファイルを選び直してください。</span
                  >{/if}</a
              >
            </li>{/each}
        </ul>
      </section>
    {/if}
    {#if id && (!settingMode || sid)}<div class="delete-area">
        <button class="btn" disabled={busy} onclick={() => void remove()}
          >{settingMode ? "この設定を削除" : "材料を削除"}</button
        >{#if !settingMode}<p class="help">
            AMSに対応づけた材料は削除できません。先にAMS画面で対応を解除してください。
          </p>{/if}
      </div>{/if}
  {/if}
</section>

<style lang="sass">
  .page-heading
    margin-top: var(--sp-3)
  h1, .settings
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
  .swatch
    display: inline-block
    width: 1em
    height: 1em
    margin-right: var(--sp-2)
    border: 1px solid var(--c-muted)
    border-radius: var(--radius-full)
</style>
