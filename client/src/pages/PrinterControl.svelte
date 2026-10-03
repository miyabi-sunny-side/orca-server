<script lang="ts">
  import { onMount } from "svelte";
  import { controlText, request, sendControl, type Printer } from "../lib/api";
  import {
    errorCode,
    hmsHelp,
    jobControls,
    liveLine,
    printerStateText,
    type Live,
  } from "../lib/queue";
  import Icon from "../lib/Icon.svelte";

  type Reading = number | null;
  type Tray = {
    id: number;
    present: boolean | null;
    material: string | null;
    temperature_min: Reading;
    temperature_max: Reading;
  };
  type Status = {
    connection: string;
    synchronized: boolean;
    print: {
      state: string | null;
      percent: Reading;
      remaining_minutes: Reading;
      error: Reading;
    };
    ams: {
      detect_on_insert: boolean | null;
      detect_on_power_up: boolean | null;
      units: { id: number; trays: Tray[] }[];
    } | null;
    external_spool: { material: string | null } | null;
    firmware: { name: string; sw_ver: string }[] | null;
    live: Live & {
      speed: { level: Reading; percent: Reading };
      fans: { part: Reading; aux: Reading; chamber: Reading };
      light: boolean | null;
      camera: { recording: boolean | null; timelapse: boolean | null };
      sdcard: boolean | null;
      wifi_signal: string | null;
      hms: string[];
      options: {
        auto_recovery: boolean | null;
        sound: boolean | null;
        remain_detection: boolean | null;
        motor_noise_calibration: boolean | null;
      };
    };
  };
  type Entry = {
    name: string;
    path: string;
    size: number;
    directory: boolean;
  };

  const id = window.location.pathname.split("/")[2];
  let printer = $state<Printer>(),
    status = $state<Status>(),
    readError = $state(""),
    busy = $state(false),
    notice = $state(""),
    stopArmed = $state(false),
    camera = $state(0),
    cameraError = $state(""),
    files = $state<{ path: string; entries: Entry[] }>(),
    filesError = $state(""),
    deleteArmed = $state(""),
    nozzle = $state(220),
    bed = $state(60),
    loadTemperature = $state(220),
    tray = $state({
      tray: 254,
      material: "PLA",
      color: "#ffffff",
      temperature_min: 190,
      temperature_max: 230,
    }),
    calibration = $state({
      bed_leveling: true,
      vibration: false,
      motor_noise: false,
    });
  const controller = new AbortController();
  const speeds = [
    [1, "静音"],
    [2, "標準"],
    [3, "スポーツ"],
    [4, "ルーディクラス"],
  ] as const;
  const can = $derived(jobControls(status?.print.state ?? null));
  const trays = $derived(
    (status?.ams?.units ?? []).flatMap((unit) =>
      unit.trays
        .filter((tray) => tray.present)
        .map((tray) => ({ ...tray, number: unit.id * 4 + tray.id })),
    ),
  );

  async function refresh() {
    try {
      const [p, s] = await Promise.all([
        request<Printer>(`/api/printers/${id}`, { signal: controller.signal }),
        request<Status>(`/api/printer/status?printer_id=${id}`, {
          signal: controller.signal,
        }),
      ]);
      printer = p;
      status = s;
      readError = "";
    } catch (e) {
      if (!controller.signal.aborted) readError = (e as Error).message;
    }
  }
  async function send(control: object) {
    busy = true;
    notice = "";
    try {
      notice = controlText(await sendControl(id, control));
    } catch (e) {
      notice = (e as Error).message;
    } finally {
      busy = false;
      void refresh();
    }
  }
  function stop() {
    if (!stopArmed) {
      stopArmed = true;
      setTimeout(() => (stopArmed = false), 4000);
      return;
    }
    stopArmed = false;
    void send({ action: "stop" });
  }
  async function list(path: string) {
    filesError = "";
    try {
      files = await request(
        `/api/printers/${id}/files?path=${encodeURIComponent(path)}`,
      );
    } catch (e) {
      filesError = (e as Error).message;
    }
  }
  async function remove(path: string) {
    if (deleteArmed !== path) {
      deleteArmed = path;
      return;
    }
    deleteArmed = "";
    try {
      await request(
        `/api/printers/${id}/files?path=${encodeURIComponent(path)}`,
        { method: "DELETE" },
      );
      if (files) await list(files.path);
    } catch (e) {
      filesError = (e as Error).message;
    }
  }
  const parent = (path: string) => path.slice(0, path.lastIndexOf("/")) || "/";
  const size = (bytes: number) =>
    bytes >= 1048576
      ? `${(bytes / 1048576).toFixed(1)} MB`
      : `${Math.ceil(bytes / 1024)} KB`;
  onMount(() => {
    void refresh();
    const timer = setInterval(() => {
      if (!document.hidden) void refresh();
    }, 2000);
    return () => {
      clearInterval(timer);
      controller.abort();
    };
  });
</script>

<svelte:head><title>本体の操作 · OrcaServer</title></svelte:head>
<section class="content" aria-label="本体の操作">
  <a href="/">キューへ</a>
  <div class="page-heading">
    <h1>{printer?.name ?? "プリンター"}</h1>
  </div>
  {#if readError}<p role="alert">{readError}</p>{/if}
  {#if status}
    <p class="summary">
      {status.synchronized
        ? [
            printerStateText(status.print.state),
            status.print.percent !== null && status.print.state !== "IDLE"
              ? `${status.print.percent}%`
              : null,
            liveLine(status.live),
          ]
            .filter(Boolean)
            .join(" · ")
        : "本体の状態を確認中"}
    </p>
    <div class="quick" role="group" aria-label="主な操作">
      {#if can.resume}<button
          class="icon-btn large"
          aria-label="再開"
          title="再開"
          disabled={busy}
          onclick={() => void send({ action: "resume" })}
          ><Icon name="play" /></button
        >{:else}<button
          class="icon-btn large"
          aria-label="一時停止"
          title="一時停止"
          disabled={busy || !can.pause}
          onclick={() => void send({ action: "pause" })}
          ><Icon name="pause" /></button
        >{/if}
      <button
        class={stopArmed ? "btn danger" : "icon-btn large"}
        aria-label={stopArmed ? "停止を確定" : "停止"}
        title="停止"
        disabled={busy || !can.stop}
        onclick={stop}
        >{#if stopArmed}停止を確定{:else}<Icon name="square" />{/if}</button
      >
      <button
        class="icon-btn large"
        aria-label={status.live.light ? "照明を消す" : "照明をつける"}
        aria-pressed={status.live.light ?? false}
        title="照明"
        disabled={busy}
        onclick={() => void send({ action: "light", on: !status!.live.light })}
        ><Icon name="lightbulb" /></button
      >
      <button
        class="icon-btn large"
        aria-label="カメラ画像を更新"
        title="カメラ"
        onclick={() => {
          cameraError = "";
          camera = Date.now();
        }}><Icon name="camera" /></button
      >
    </div>
    {#if notice}<p class="caption" role="status">{notice}</p>{/if}
    {#if camera}
      <img
        class="camera"
        src={`/api/printers/${id}/camera?t=${camera}`}
        alt="カメラ画像"
        onerror={() => (cameraError = "カメラ画像を取得できませんでした")}
      />
      {#if cameraError}<p role="alert">{cameraError}</p>{/if}
    {/if}
    {#if status.live.hms.length || status.print.error}
      <ul class="codes" aria-label="本体のエラー">
        {#if status.print.error}<li>
            print_error {errorCode(status.print.error)}
            <button
              class="btn"
              disabled={busy}
              onclick={() =>
                void send({ action: "clear_error", code: status!.print.error })}
              >消去</button
            >
          </li>{/if}
        {#each status.live.hms as code (code)}<li>
            HMS {code}
            {#if hmsHelp(code)}<a
                class="icon-btn"
                href={hmsHelp(code)}
                target="_blank"
                rel="noopener noreferrer"
                aria-label={`HMS ${code} の公式解説（別タブ）`}
                ><Icon name="external-link" /></a
              >{/if}
          </li>{/each}
      </ul>
    {/if}

    <details>
      <summary>温度・ファン・速度</summary>
      <div class="grid">
        <label class="field"
          ><span>ノズル（℃）</span><input
            type="number"
            min="0"
            max="300"
            bind:value={nozzle}
          /></label
        >
        <button
          class="btn"
          disabled={busy}
          onclick={() =>
            void send({ action: "nozzle_temperature", celsius: nozzle })}
          >適用</button
        >
        <label class="field"
          ><span>ベッド（℃）</span><input
            type="number"
            min="0"
            max="120"
            bind:value={bed}
          /></label
        >
        <button
          class="btn"
          disabled={busy}
          onclick={() => void send({ action: "bed_temperature", celsius: bed })}
          >適用</button
        >
      </div>
      {#each [["part", "部品冷却"], ["aux", "補助"], ["chamber", "チャンバー"]] as const as [fan, label] (fan)}
        <label class="field"
          ><span>{label}ファン {status.live.fans[fan] ?? "—"}%</span><input
            type="range"
            min="0"
            max="100"
            step="10"
            value={status.live.fans[fan] ?? 0}
            disabled={busy}
            onchange={(e) =>
              void send({
                action: "fan",
                fan,
                percent: Number(e.currentTarget.value),
              })}
          /></label
        >
      {/each}
      <label class="field"
        ><span>印刷速度</span><select
          value={status.live.speed.level ?? 2}
          disabled={busy}
          onchange={(e) =>
            void send({
              action: "speed",
              level: Number(e.currentTarget.value),
            })}
          >{#each speeds as [level, label] (level)}<option value={level}
              >{label}</option
            >{/each}</select
        ></label
      >
    </details>

    <details>
      <summary>移動</summary>
      <div class="pad" role="group" aria-label="ヘッドとベッドの移動">
        <button
          class="icon-btn large up"
          aria-label="Y +10mm"
          disabled={busy}
          onclick={() => void send({ action: "move", axis: "Y", mm: 10 })}
          ><Icon name="arrow-up" /></button
        >
        <button
          class="icon-btn large left"
          aria-label="X -10mm"
          disabled={busy}
          onclick={() => void send({ action: "move", axis: "X", mm: -10 })}
          ><Icon name="arrow-left" /></button
        >
        <button
          class="icon-btn large center"
          aria-label="ホーム"
          disabled={busy}
          onclick={() => void send({ action: "home" })}
          ><Icon name="home" /></button
        >
        <button
          class="icon-btn large right"
          aria-label="X +10mm"
          disabled={busy}
          onclick={() => void send({ action: "move", axis: "X", mm: 10 })}
          ><Icon name="arrow-right" /></button
        >
        <button
          class="icon-btn large down"
          aria-label="Y -10mm"
          disabled={busy}
          onclick={() => void send({ action: "move", axis: "Y", mm: -10 })}
          ><Icon name="arrow-down" /></button
        >
      </div>
      <div class="row">
        {#each [[-10, "Z -10mm"], [-1, "Z -1mm"], [1, "Z +1mm"], [10, "Z +10mm"]] as const as [mm, label] (mm)}<button
            class="btn"
            disabled={busy}
            onclick={() => void send({ action: "move", axis: "Z", mm })}
            >{label}</button
          >{/each}
      </div>
      <div class="row">
        <button
          class="btn"
          disabled={busy}
          onclick={() => void send({ action: "extrude", mm: 10 })}
          >押出 10mm</button
        >
        <button
          class="btn"
          disabled={busy}
          onclick={() => void send({ action: "extrude", mm: -10 })}
          >引戻し 10mm</button
        >
      </div>
    </details>

    <details>
      <summary>給材</summary>
      <label class="field"
        ><span>ロード・アンロードのノズル温度（℃）</span><input
          type="number"
          min="150"
          max="300"
          bind:value={loadTemperature}
        /></label
      >
      <ul class="trays">
        {#each trays as tray (tray.number)}<li>
            AMS {Math.floor(tray.number / 4) + 1}-{(tray.number % 4) + 1} ·
            {tray.material ?? "材料不明"}
            <button
              class="btn"
              disabled={busy}
              onclick={() =>
                void send({
                  action: "load",
                  tray: tray.number,
                  celsius: loadTemperature,
                })}>ロード</button
            >
            <button
              class="btn"
              disabled={busy}
              onclick={() =>
                void send({ action: "read_tray", tray: tray.number })}
              >再読取</button
            >
          </li>{/each}
        <li>
          外部スプール · {status.external_spool?.material ?? "材料未設定"}
          <button
            class="btn"
            disabled={busy}
            onclick={() =>
              void send({
                action: "load",
                tray: 254,
                celsius: loadTemperature,
              })}>ロード</button
          >
        </li>
      </ul>
      <div class="row">
        <button
          class="btn"
          disabled={busy}
          onclick={() =>
            void send({ action: "unload", celsius: loadTemperature })}
          >アンロード</button
        >
        <button
          class="btn"
          disabled={busy}
          onclick={() => void send({ action: "ams", step: "resume" })}
          >AMS 再試行</button
        >
        <button
          class="btn"
          disabled={busy}
          onclick={() => void send({ action: "ams", step: "done" })}
          >AMS 完了</button
        >
      </div>
      <fieldset class="tray-setting">
        <legend>本体に伝える材料</legend>
        <label class="field"
          ><span>対象</span><select bind:value={tray.tray}
            ><option value={254}>外部スプール</option
            >{#each trays as t (t.number)}<option value={t.number}
                >AMS {Math.floor(t.number / 4) + 1}-{(t.number % 4) + 1}</option
              >{/each}</select
          ></label
        >
        <label class="field"
          ><span>材質</span><input
            bind:value={tray.material}
            maxlength="16"
          /></label
        >
        <label class="field"
          ><span>色</span><input type="color" bind:value={tray.color} /></label
        >
        <label class="field"
          ><span>最低（℃）</span><input
            type="number"
            min="150"
            max="300"
            bind:value={tray.temperature_min}
          /></label
        >
        <label class="field"
          ><span>最高（℃）</span><input
            type="number"
            min="150"
            max="300"
            bind:value={tray.temperature_max}
          /></label
        >
        <button
          class="btn"
          disabled={busy}
          onclick={() =>
            void send({
              action: "tray_setting",
              ...tray,
              color: `${tray.color.slice(1).toUpperCase()}FF`,
              profile_id: "",
            })}>適用</button
        >
      </fieldset>
    </details>

    <details>
      <summary>オプション</summary>
      {#snippet toggle(label: string, value: boolean | null, control: object)}
        <label class="check"
          ><input
            type="checkbox"
            checked={value ?? false}
            disabled={busy || value === null}
            onchange={() => void send(control)}
          />{label}{#if value === null}<span class="caption">（未報告）</span
            >{/if}</label
        >
      {/snippet}
      {@render toggle("自動復旧", status.live.options.auto_recovery, {
        action: "auto_recovery",
        on: !status.live.options.auto_recovery,
      })}
      {#if status.live.options.sound !== null}{@render toggle(
          "通知音",
          status.live.options.sound,
          { action: "sound", on: !status.live.options.sound },
        )}{/if}
      {@render toggle("録画", status.live.camera.recording, {
        action: "recording",
        on: !status.live.camera.recording,
      })}
      {@render toggle("タイムラプス", status.live.camera.timelapse, {
        action: "timelapse",
        on: !status.live.camera.timelapse,
      })}
      {#if status.ams}
        {@const reading = {
          on_insert: status.ams.detect_on_insert ?? false,
          on_power_up: status.ams.detect_on_power_up ?? false,
          remain: status.live.options.remain_detection ?? false,
        }}
        {@render toggle("挿入時にAMSを読取", status.ams.detect_on_insert, {
          action: "ams_reading",
          ...reading,
          on_insert: !reading.on_insert,
        })}
        {@render toggle("起動時にAMSを読取", status.ams.detect_on_power_up, {
          action: "ams_reading",
          ...reading,
          on_power_up: !reading.on_power_up,
        })}
        {@render toggle("残量を推定", status.live.options.remain_detection, {
          action: "ams_reading",
          ...reading,
          remain: !reading.remain,
        })}
      {/if}
    </details>

    <details>
      <summary>キャリブレーション</summary>
      <label class="check"
        ><input
          type="checkbox"
          bind:checked={calibration.bed_leveling}
        />ベッドレベリング</label
      >
      <label class="check"
        ><input
          type="checkbox"
          bind:checked={calibration.vibration}
        />振動補正</label
      >
      {#if status.live.options.motor_noise_calibration}<label class="check"
          ><input
            type="checkbox"
            bind:checked={calibration.motor_noise}
          />モーターノイズ</label
        >{/if}
      <button
        class="btn"
        disabled={busy ||
          !(
            calibration.bed_leveling ||
            calibration.vibration ||
            calibration.motor_noise
          )}
        onclick={() => void send({ action: "calibrate", ...calibration })}
        >開始</button
      >
    </details>

    <details ontoggle={(e) => e.currentTarget.open && !files && void list("/")}>
      <summary>ファイル</summary>
      {#if filesError}<p role="alert">{filesError}</p>{/if}
      {#if files}
        <p class="caption">
          {files.path}
          {#if files.path !== "/"}<button
              class="btn"
              onclick={() => void list(parent(files!.path))}>上へ</button
            >{/if}
        </p>
        <ul class="files">
          {#each files.entries as entry (entry.path)}<li>
              {#if entry.directory}<button
                  class="btn"
                  onclick={() => void list(entry.path)}>{entry.name}/</button
                >{:else}<span class="name">{entry.name}</span>
                <span class="caption">{size(entry.size)}</span>
                <a
                  class="icon-btn"
                  href={`/api/printers/${id}/files/content?path=${encodeURIComponent(entry.path)}`}
                  aria-label={`${entry.name}をダウンロード`}
                  ><Icon name="download" /></a
                >
                <button
                  class={deleteArmed === entry.path ? "btn danger" : "icon-btn"}
                  aria-label={deleteArmed === entry.path
                    ? `${entry.name}の削除を確定`
                    : `${entry.name}を削除`}
                  onclick={() => void remove(entry.path)}
                  >{#if deleteArmed === entry.path}削除を確定{:else}<Icon
                      name="trash"
                    />{/if}</button
                >{/if}
            </li>{:else}<li class="caption">ファイルはありません</li>{/each}
        </ul>
      {/if}
    </details>

    <details>
      <summary>本体の情報</summary>
      <ul>
        {#each status.firmware ?? [] as module (module.name)}<li>
            {module.name}
            {module.sw_ver}
          </li>{/each}
        <li>Wi-Fi {status.live.wifi_signal ?? "—"}</li>
        <li>
          SDカード {status.live.sdcard === null
            ? "—"
            : status.live.sdcard
              ? "あり"
              : "なし"}
        </li>
      </ul>
    </details>
  {:else if !readError}<p role="status">読み込んでいます…</p>{/if}
</section>

<style lang="sass">
  .summary
    margin: 0 0 var(--sp-2)
  .quick, .row
    display: flex
    flex-wrap: wrap
    align-items: center
    gap: var(--sp-2)
  .row
    margin-top: var(--sp-2)
  details
    margin-top: var(--sp-3)
    border-top: 1px solid var(--c-border)
    padding-top: var(--sp-2)
  summary
    cursor: pointer
    min-height: 44px
    display: flex
    align-items: center
  .grid
    display: grid
    grid-template-columns: 1fr auto
    align-items: end
    gap: var(--sp-2)
  .pad
    display: grid
    grid-template-columns: repeat(3, 44px)
    grid-template-areas: ". up ." "left center right" ". down ."
    gap: var(--sp-1)
    .up
      grid-area: up
    .left
      grid-area: left
    .center
      grid-area: center
    .right
      grid-area: right
    .down
      grid-area: down
  .camera
    display: block
    width: 100%
    max-width: 640px
    margin-top: var(--sp-2)
    border-radius: var(--radius-sm)
  .codes, .trays, .files
    list-style: none
    padding: 0
    li
      display: flex
      flex-wrap: wrap
      align-items: center
      gap: var(--sp-2)
      min-height: 44px
  .name
    overflow-wrap: anywhere
  .tray-setting
    display: grid
    gap: var(--sp-2)
    margin: var(--sp-3) 0 0
    border: 1px solid var(--c-border)
    border-radius: var(--radius-sm)
  .check
    display: flex
    align-items: center
    gap: var(--sp-2)
    min-height: 44px
</style>
