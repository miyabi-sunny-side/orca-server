import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, reset, visit } from "../testing/page";
import { queueFixture } from "../testing/queue";

afterEach(reset);

const live = {
  temperatures: {
    nozzle: 212.4,
    nozzle_target: 220,
    bed: 60,
    bed_target: 60,
    chamber: null,
  },
  layer: { current: null, total: null },
};

/** The queue's printer printing, and the printer answering every control. */
function printing() {
  const f = queueFixture();
  Object.assign(f.q.printer, { live });
  const controls: any[] = [];
  f.api.route(({ method, path, body }) => {
    if (path === "/api/printers/p1/control" && method === "POST") {
      controls.push(body);
      return { json: { reply: "success" } };
    }
  });
  return { ...f, controls };
}
const notice = (text: string) =>
  waitFor(() =>
    expect(
      screen.getAllByRole("status").some((s) => s.textContent?.includes(text)),
    ).toBe(true),
  );

it("the plate's start options are one collapsed section of the editor", async () => {
  const id = "11111111-1111-4111-8111-111111111111";
  const { api } = printing();
  const plate = {
    id,
    version: 1,
    name: "開始の条件",
    models: [{ id: "m", name: "cube.stl", source: "cube.stl", quantity: 3 }],
    conditions: { filament_id: "petg" },
  };
  api.route(({ method, path, body }) => {
    if (path === `/api/plates/${id}`)
      return {
        json: method === "PUT" ? { ...plate, ...body, version: 2 } : plate,
      };
    if (path === "/api/default-settings")
      return {
        json: {
          default_printer_id: "p1",
          conditions: { filament_id: null },
          infill_patterns: [],
          reason: null,
        },
      };
  });
  visit(`/plates/${id}?edit=1`);
  const start = await waitFor(() => {
    const found = [...document.querySelectorAll("details")].find(
      (d) => d.querySelector("summary")?.textContent?.trim() === "印刷開始",
    );
    expect(found).toBeTruthy();
    return found!;
  });
  expect(start.open).toBe(false);
  await fireEvent.click(start.querySelector("summary")!);
  const option = (name: string) =>
    within(start).getByLabelText<HTMLInputElement>(name);
  expect(option("ベッドレベリング").checked).toBe(true);
  expect(option("振動補正").checked).toBe(false);
  await fireEvent.click(option("タイムラプス"));
  await click("保存");
  await waitFor(() =>
    expect(api.to("PUT", `/api/plates/${id}`)).toHaveLength(1),
  );
  expect(
    api.to("PUT", `/api/plates/${id}`)[0].body.conditions.start_options,
  ).toEqual({
    bed_leveling: true,
    flow_calibration: true,
    timelapse: false,
    vibration_calibration: false,
  });
});

it("print controls on the queue act on the printer", async () => {
  const { q, controls } = printing();
  visit("/");
  expect(
    await screen.findByText(/ノズル 212\/220℃ · ベッド 60\/60℃/),
  ).toBeTruthy();
  await click("一時停止");
  await waitFor(() => expect(controls).toEqual([{ action: "pause" }]));
  await notice("本体が受け付けました");
  q.printer.print.state = "PAUSE";
  // The queue shows the printer's report from its next reading.
  await waitFor(() => screen.getByRole("button", { name: "再開" }), {
    timeout: 3000,
  });
  await click("再開");
  await waitFor(() => expect(controls.at(-1)).toEqual({ action: "resume" }));
  q.printer.print.state = "RUNNING";
  // Stopping takes a second, explicit press.
  await click("停止");
  expect(controls).toHaveLength(2);
  await click("停止を確定");
  await waitFor(() => expect(controls.at(-1)).toEqual({ action: "stop" }));
  expect(
    screen.getByRole("link", { name: "本体の操作" }).getAttribute("href"),
  ).toBe("/printers/p1/control");
}, 10_000);

it("the printer page keeps routine actions as icons and everything else collapsed", async () => {
  const { api, controls } = printing();
  let files = [
    { name: "timelapse", path: "/timelapse", size: 0, directory: true },
  ];
  const deleted: string[] = [];
  api.route(({ method, path, url }) => {
    if (path === "/api/printers/p1")
      return {
        json: {
          id: "p1",
          name: "P1S",
          machine_profile_key: "Bambu Lab P1S 0.4 nozzle",
          status: { connection: "connected" },
        },
      };
    if (path === "/api/printer/status")
      return {
        json: {
          connection: "connected",
          synchronized: true,
          print: {
            state: "FAILED",
            percent: null,
            remaining_minutes: null,
            error: 0,
          },
          ams: { detect_on_insert: true, detect_on_power_up: true, units: [] },
          external_spool: { material: null },
          firmware: null,
          descriptions: {
            "0300_0D00_0001_0004": "ヒートベッドの原点復帰に異常があります。",
          },
          live: {
            ...live,
            speed: { level: 2, percent: 100 },
            fans: { part: 0, aux: 0, chamber: 0 },
            light: true,
            camera: { recording: false, timelapse: false },
            sdcard: true,
            wifi_signal: "-40dBm",
            hms: ["0300_0D00_0001_0004"],
            options: {
              auto_recovery: true,
              sound: true,
              remain_detection: true,
              motor_noise_calibration: false,
            },
          },
        },
      };
    if (path === "/api/printers/p1/files") {
      const at = url.searchParams.get("path");
      if (method === "DELETE") {
        deleted.push(at!);
        files = [];
        return { status: 204 };
      }
      return {
        json: {
          path: at,
          entries:
            at === "/timelapse"
              ? files.length
                ? [
                    {
                      name: "video_1.mp4",
                      path: "/timelapse/video_1.mp4",
                      size: 2048,
                      directory: false,
                    },
                  ]
                : []
              : files,
        },
      };
    }
  });
  visit("/printers/p1/control");
  expect(await screen.findByRole("heading", { level: 1 })).toBeTruthy();
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "照明を消す" })).toBeTruthy(),
  );
  expect(document.querySelectorAll("details[open]")).toHaveLength(0);
  await click("照明を消す");
  await waitFor(() => expect(controls).toHaveLength(1));
  await notice("本体が受け付けました");
  await click("カメラ画像を更新");
  const image = await screen.findByRole("img", { name: "カメラ画像" });
  expect(image.getAttribute("src")).toMatch(
    /^\/api\/printers\/p1\/camera\?t=\d+$/,
  );
  expect(
    screen
      .getByRole("link", { name: /HMS 0300_0D00_0001_0004 の公式解説/ })
      .getAttribute("href"),
  ).toMatch(/e=03000D0000010004/);
  expect(
    screen
      .getByRole("list", { name: "本体のエラー" })
      .textContent?.replace(/\s+/g, " "),
  ).toContain(
    "HMS 0300_0D00_0001_0004 ヒートベッドの原点復帰に異常があります。",
  );
  const section = (name: string) =>
    [...document.querySelectorAll("details")].find((d) =>
      d.querySelector("summary")?.textContent?.includes(name),
    )!;
  await fireEvent.click(section("ファイル").querySelector("summary")!);
  await click("timelapse/", section("ファイル"));
  const download = await within(section("ファイル")).findByRole("link", {
    name: "video_1.mp4をダウンロード",
  });
  expect(download.getAttribute("href")).toMatch(
    /path=%2Ftimelapse%2Fvideo_1.mp4/,
  );
  await click("video_1.mp4を削除", section("ファイル"));
  expect(deleted).toEqual([]);
  await click("video_1.mp4の削除を確定", section("ファイル"));
  expect(
    await within(section("ファイル")).findByText("ファイルはありません"),
  ).toBeTruthy();
  expect(deleted).toEqual(["/timelapse/video_1.mp4"]);
  await fireEvent.click(section("給材").querySelector("summary")!);
  await fireEvent.input(within(section("給材")).getByLabelText("材質"), {
    target: { value: "PETG" },
  });
  await click("適用", section("給材"));
  await waitFor(() =>
    expect(JSON.stringify(controls.at(-1))).toContain("PETG"),
  );
});
