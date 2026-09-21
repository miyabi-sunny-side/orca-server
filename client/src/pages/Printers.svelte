<script lang="ts">
  import { onMount } from "svelte";
  import {
    request,
    type Printer,
    type DefaultSettings,
    type Profiles,
    type Machine,
  } from "../lib/api";
  const part = window.location.pathname.split("/")[2];
  const editing = !!part;
  const id = part === "new" ? "" : part;
  let printers = $state<Printer[]>([]);
  let machines = $state<Machine[]>([]);
  let defaultId = $state("");
  let defaultSaved = $state(false);
  let profiles = $state<Profiles>();
  let loading = $state(true);
  let busy = $state(false);
  let error = $state("");
  let profileError = $state("");
  let settings = $state({
    name: "",
    host: "",
    serial: "",
    access_code: "",
    tls_certificate: "",
    machine_profile_key: "",
    default_process_profile_key: "",
    bed_type: "Textured PEI Plate",
    nozzle_material: "unknown",
    mqtt_port: 8883,
    ftps_port: 990,
    start_timeout_secs: 600,
  });
  let original = $state<Printer>();
  const controller = new AbortController();
  let profileSequence = 0;
  async function loadProfiles(reset = false) {
    const ticket = ++profileSequence;
    profiles = undefined;
    profileError = "";
    if (!settings.machine_profile_key) return;
    try {
      const value = await request<Profiles>(
        `/api/slicer/profiles?machine=${encodeURIComponent(settings.machine_profile_key)}`,
        { signal: controller.signal },
      );
      if (controller.signal.aborted || ticket !== profileSequence) return;
      profiles = value;
      if (reset) {
        settings.default_process_profile_key = value.defaults.process;
        settings.bed_type = value.defaults.bed;
      }
    } catch (cause) {
      if (!controller.signal.aborted && ticket === profileSequence)
        profileError = (cause as Error).message;
    }
  }
  async function load() {
    loading = true;
    error = "";
    try {
      if (editing) {
        machines = await request<Machine[]>("/api/printers/profiles", {
          signal: controller.signal,
        });
        if (id) {
          original = await request<Printer>(`/api/printers/${id}`, {
            signal: controller.signal,
          });
          for (const key of Object.keys(
            settings,
          ) as (keyof typeof settings)[]) {
            if (key !== "access_code" && key !== "tls_certificate")
              (settings as Record<string, unknown>)[key] = original[key];
          }
        }
        await loadProfiles();
      } else {
        const [devices, defaults] = await Promise.all([
          request<Printer[]>("/api/printers", { signal: controller.signal }),
          request<DefaultSettings>("/api/default-settings", {
            signal: controller.signal,
          }),
        ]);
        printers = devices;
        defaultId = defaults.default_printer_id ?? "";
      }
    } catch (cause) {
      if (!controller.signal.aborted) error = (cause as Error).message;
    } finally {
      if (!controller.signal.aborted) loading = false;
    }
  }
  onMount(() => {
    void load();
    const timer = setInterval(async () => {
      if (editing || document.hidden || busy || loading) return;
      try {
        printers = await request<Printer[]>("/api/printers", {
          signal: controller.signal,
        });
        error = "";
      } catch (cause) {
        if (!controller.signal.aborted) error = (cause as Error).message;
      }
    }, 5000);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  });
  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (busy || !profiles) return;
    busy = true;
    error = "";
    try {
      await request(`/api/printers${id ? `/${id}` : ""}`, {
        method: id ? "PUT" : "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(settings),
      });
      window.location.assign("/printers");
    } catch (cause) {
      error = (cause as Error).message;
      busy = false;
    }
  }
  async function chooseDefault(event: Event) {
    const chosen = (event.currentTarget as HTMLSelectElement).value;
    busy = true;
    error = "";
    defaultSaved = false;
    try {
      await request("/api/default-settings", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ default_printer_id: chosen }),
      });
      defaultId = chosen;
      defaultSaved = true;
    } catch (cause) {
      error = (cause as Error).message;
      (event.target as HTMLSelectElement).value = defaultId;
    } finally {
      busy = false;
    }
  }
  async function remove() {
    if (busy || !id || !window.confirm(`「${settings.name}」を削除しますか？`))
      return;
    busy = true;
    error = "";
    try {
      await request(`/api/printers/${id}`, { method: "DELETE" });
      window.location.assign("/printers");
    } catch (cause) {
      error = (cause as Error).message;
      busy = false;
    }
  }
  const connection: Record<string, string> = {
    connected: "接続済み",
    connecting: "接続中",
    disconnected: "未接続",
    synchronizing: "状態を取得中",
    stale: "状態を再確認中",
    unconfigured: "設定を確認",
  };
</script>

<svelte:head
  ><title
    >{editing ? (id ? "プリンターを編集" : "プリンターを追加") : "プリンター"} · OrcaServer</title
  ></svelte:head
>
<section class="content" aria-label="プリンター管理">
  {#if editing}<a class="back-link" href="/printers">プリンター一覧へ</a>{/if}
  <div class="page-heading">
    <h1>
      {editing ? (id ? "プリンターを編集" : "プリンターを追加") : "プリンター"}
    </h1>
    {#if !editing}<a class="btn primary" href="/printers/new">追加</a>{/if}
  </div>
  {#if error}<div class="notice">
      <p role="alert">{error}</p>
      {#if !busy}<button class="btn" onclick={() => void load()}
          >読み直す</button
        >{/if}
    </div>{/if}
  {#if loading}<p class="state" role="status">読み込んでいます…</p>
  {:else if !editing}
    {#if printers.length === 0}<p class="state">
        プリンターを追加すると、接続状態の確認と印刷ができます。
      </p>{/if}
    {#if printers.length}
      <label class="field"
        ><span>新規プレートの初期値に使うプリンター</span>
        <select
          value={defaultId}
          disabled={busy}
          onchange={(event) => void chooseDefault(event)}
        >
          <option value="" disabled>選択してください</option>
          {#each printers as printer}<option value={printer.id}
              >{printer.name}</option
            >{/each}
        </select>
      </label>
      <p class="caption">
        この機器の設定とAMS材料を初期入力に使います。保存済みの条件は変わりません。
      </p>
      {#if defaultSaved}<p role="status">
          初期値に使うプリンターを保存しました。
        </p>{/if}
    {/if}
    <ul class="plate-list">
      {#each printers as printer (printer.id)}<li class="printer-card">
          <a class="printer-title" href={`/printers/${printer.id}`}
            >{printer.name}</a
          >
          <p class="caption">
            {printer.machine?.model ?? printer.machine_profile_key} · {printer
              .machine?.nozzle_diameter ?? "—"} mm
          </p>
          <p class="caption">
            {printer.nozzle_material === "hardened_steel"
              ? "焼入れ鋼"
              : printer.nozzle_material === "stainless_steel"
                ? "ステンレス"
                : "材質未確認"}
          </p>
          <p class="connection">
            {connection[printer.status.connection] ?? "状態不明"}
          </p>
          {#if printer.configuration_error}<p role="alert">
              設定したプロファイルまたは接続情報を確認してください。
            </p>{/if}
          <div class="actions">
            <a class="btn" href={`/printers/${printer.id}/ams`}>AMSの材料</a>
            <a class="btn" href={`/queue?printer_id=${printer.id}`}
              >印刷キュー</a
            ><a href={`/printers/${printer.id}`}>設定を編集</a>
          </div>
        </li>{/each}
    </ul>
  {:else}
    <form onsubmit={save}>
      <fieldset disabled={busy}>
        <label class="field"
          ><span>名前</span><input
            bind:value={settings.name}
            required
            maxlength="120"
            placeholder="例: 作業部屋のP1S"
          /></label
        >
        <h2>機種とノズル</h2>
        <label class="field"
          ><span>機種・装着ノズル径</span><select
            bind:value={settings.machine_profile_key}
            required
            onchange={() => void loadProfiles(true)}
          >
            <option value="" disabled>構成を選択</option>
            {#if settings.machine_profile_key && !machines.some((m) => m.key === settings.machine_profile_key)}<option
                value={settings.machine_profile_key}
                >未対応: {settings.machine_profile_key}</option
              >{/if}
            {#each machines as machine}<option value={machine.key}
                >{machine.model} / {machine.nozzle_diameter} mm</option
              >{/each}
          </select></label
        >
        <label class="field"
          ><span>ノズル材質</span><select bind:value={settings.nozzle_material}
            ><option value="unknown">未確認</option><option
              value="stainless_steel">ステンレス</option
            ><option value="hardened_steel">焼入れ鋼</option></select
          ></label
        >
        <p class="help">
          装着したノズルを登録します。交換後はプリンター本体の設定も合わせてください。
        </p>
        {#if original?.status.nozzle_diameter}<p class="caption">
            本体の申告値: {original.status.nozzle_diameter} mm{original.status
              .nozzle_material
              ? ` / ${original.status.nozzle_material}`
              : ""}
          </p>{/if}
        {#if profileError}<div class="notice">
            <p role="alert">{profileError}</p>
            <button
              class="btn"
              type="button"
              onclick={() => void loadProfiles()}>プロファイルを再読込み</button
            >
          </div>{/if}
        <h2>新規プレートの初期値</h2>
        <p class="caption">
          印刷中も変更できます。保存済みの条件や進行中の印刷には反映しません。
        </p>
        <label class="field"
          ><span>既定の工程</span><select
            bind:value={settings.default_process_profile_key}
            required
            disabled={!profiles}
          >
            {#if settings.default_process_profile_key && !profiles?.processes.includes(settings.default_process_profile_key)}<option
                value={settings.default_process_profile_key}
                >要確認: {settings.default_process_profile_key}</option
              >{/if}
            {#each profiles?.processes ?? [] as process}<option
                >{process}</option
              >{/each}
          </select></label
        >
        <label class="field"
          ><span>プレート種類</span><select bind:value={settings.bed_type}
            >{#each profiles?.beds ?? [settings.bed_type] as bed}<option
                >{bed}</option
              >{/each}</select
          ></label
        >
        <h2>LAN接続</h2>
        <label class="field"
          ><span>IPアドレス</span><input
            bind:value={settings.host}
            required
            placeholder="192.168.1.50"
            autocomplete="off"
          /></label
        >
        <label class="field"
          ><span>シリアル番号</span><input
            bind:value={settings.serial}
            required
            maxlength="64"
            autocomplete="off"
          /></label
        >
        <label class="field"
          ><span>LANアクセスコード</span><input
            type="password"
            bind:value={settings.access_code}
            required={!id}
            maxlength="128"
            autocomplete="new-password"
          /></label
        >
        <label class="field"
          ><span>TLS証明書（PEM）</span><textarea
            bind:value={settings.tls_certificate}
            required={!id}
            rows="5"
            spellcheck={false}
            placeholder="-----BEGIN CERTIFICATE-----"></textarea></label
        >
        <p class="help">
          接続先の証明書を照合します。{id
            ? "アクセスコードと証明書は、空欄のまま保存すると現在の値を維持します。"
            : "プリンターから取得し、接続先を確認した証明書を入力してください。"}
        </p>
        <details>
          <summary>接続の詳細</summary>
          <label class="field"
            ><span>MQTTポート</span><input
              type="number"
              min="1"
              max="65535"
              bind:value={settings.mqtt_port}
              required
            /></label
          >
          <label class="field"
            ><span>FTPSポート</span><input
              type="number"
              min="1"
              max="65535"
              bind:value={settings.ftps_port}
              required
            /></label
          >
          <label class="field"
            ><span>印刷開始の確認待ち時間（秒）</span><input
              type="number"
              min="1"
              max="3600"
              bind:value={settings.start_timeout_secs}
              required
            /></label
          >
        </details>
      </fieldset>
      {#if busy}<p role="status">保存しています…</p>{/if}
      <div class="actions">
        <button class="btn primary" disabled={busy || !profiles}>保存</button><a
          class="btn"
          href="/printers">キャンセル</a
        >
      </div>
    </form>
    {#if id}<div class="delete-area">
        <button class="btn" disabled={busy} onclick={() => void remove()}
          >プリンターを削除</button
        >
        <p class="help">印刷中やキューにジョブがある機器は削除できません。</p>
      </div>{/if}
  {/if}
</section>

<style lang="sass">
  .back-link
    display: inline-block
    margin-bottom: var(--sp-3)
  .printer-card
    padding: var(--sp-3)
    border: 1px solid var(--c-border)
    border-radius: var(--radius-md)
    background: var(--c-surface-raised)
    overflow-wrap: anywhere
  .printer-title
    font-size: var(--fs-md)
    font-weight: 600
  .printer-card p
    margin: var(--sp-1) 0
  .connection
    font-size: var(--fs-sm)
  fieldset
    border: 0
    padding: 0
    margin: 0
    min-width: 0
  h2
    margin-top: var(--sp-5)
  textarea
    width: 100%
    resize: vertical
    font-family: monospace
  details
    margin: var(--sp-4) 0
  summary
    cursor: pointer
    margin-bottom: var(--sp-3)
  .delete-area
    margin-top: var(--sp-5)
    padding-top: var(--sp-4)
    border-top: 1px solid var(--c-border)
</style>
