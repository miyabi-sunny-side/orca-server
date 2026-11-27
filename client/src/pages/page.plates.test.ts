import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, press, reload, reset, visit, went } from "../testing/page";
import { gate, server, type Route } from "../testing/server";

afterEach(reset);

const machine = "Bambu Lab P1S 0.4 nozzle";
const rows = () => [...document.querySelectorAll<HTMLElement>(".plate-row")];
const dialog = () => screen.getByRole("dialog");
const inDialog = (name: string) =>
  within(dialog())
    .getAllByRole<HTMLButtonElement>("button", { name })
    .find((b) => !b.classList.contains("scrim"))!;

/** Two saved plates, one printer and a queue that admits them until `hold`. */
function plates(title?: string) {
  let saved = [1, 2].map((i) => ({
    id: `10000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
    version: 1,
    name: title ?? `保存プレート ${i}`,
    models: [
      { id: "model", name: "cube.stl", source: "cube.stl", quantity: 1 },
    ],
    conditions: { filament_id: "white" },
  }));
  const printers = [{ id: "p1", name: "P1S", machine_profile_key: machine }];
  let allowed = true;
  const commands: any[] = [];
  const deletes: string[] = [];
  const api = server(({ method, path, url, body }) => {
    if (path === "/api/plates") return { json: saved };
    if (path.startsWith("/api/plates/")) {
      if (method === "DELETE") {
        deletes.push(path);
        saved = saved.filter((p) => !path.endsWith(p.id));
        return { status: 204 };
      }
      return { json: saved.find((p) => path.endsWith(p.id)) };
    }
    if (path === "/api/printers") return { json: printers };
    if (path === "/api/queue") {
      if (method === "POST")
        commands.push({
          printer: url.searchParams.get("printer_id"),
          command: body,
        });
      return {
        json: {
          epoch: "e",
          generation: commands.length,
          request_id: "request",
          waiting: [],
          current: null,
          admission: {
            allowed,
            plate_version: 1,
            reason: allowed
              ? null
              : "No confirmed AMS slot contains the plate material",
          },
        },
      };
    }
  });
  return { api, printers, commands, deletes, hold: () => (allowed = false) };
}

it("right click, keyboard, admission, edit and logical deletion", async () => {
  const { printers, commands, hold } = plates();
  visit("/plates");
  await waitFor(() => expect(rows()).toHaveLength(2));
  const row = rows()[0];
  await fireEvent.contextMenu(row);
  await waitFor(() => expect(inDialog("キュー追加").disabled).toBe(false));
  expect(screen.queryByLabelText("追加先のプリンター")).toBeNull();
  await press("Escape");
  await waitFor(() => expect(document.activeElement).toBe(row));
  await press("F10", row, { shiftKey: true });
  expect(dialog()).toBeTruthy();
  await fireEvent.click(document.querySelector(".scrim")!);
  expect(screen.queryByRole("dialog")).toBeNull();
  printers.push({ id: "p2", name: "P1S 2", machine_profile_key: machine });
  await fireEvent.contextMenu(row);
  await fireEvent.change(await screen.findByLabelText("追加先のプリンター"), {
    target: { value: "p2" },
  });
  await click("キュー追加", dialog());
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(commands).toHaveLength(1);
  expect(commands[0].printer).toBe("p2");
  expect(commands[0].command.action.type).toBe("add");
  hold();
  await fireEvent.contextMenu(row);
  expect(
    await within(dialog()).findByText(
      "この実機のAMSに指定材料の装填を確認できません。AMSの材料を確認してください。",
    ),
  ).toBeTruthy();
  expect(inDialog("キュー追加").disabled).toBe(true);
  // Disabled controls are skipped by the existing modal's focus trap.
  const close = within(dialog())
    .getAllByRole("button", { name: "閉じる" })
    .find((b) => !b.classList.contains("scrim"))!;
  close.focus();
  await press("Tab", close, { shiftKey: true });
  expect(document.activeElement).not.toBe(inDialog("キュー追加"));
  inDialog("削除").focus();
  await fireEvent.click(inDialog("削除"));
  expect(dialog().getAttribute("aria-label")).toBe("プレートを削除");
  await fireEvent.click(inDialog("削除"));
  await waitFor(() => expect(rows()).toHaveLength(1));
  await waitFor(() => expect(document.activeElement).toBe(rows()[0]));
  await press("F10", rows()[0], { shiftKey: true });
  await fireEvent.click(inDialog("編集"));
  expect(went).toHaveLength(1);
  expect(went[0]).toMatch(/\/plates\/.+\?edit=1$/);
});

it("long press opens the same menu, release does not navigate, scroll cancels it", async () => {
  plates();
  visit("/plates");
  await waitFor(() => expect(rows()).toHaveLength(2));
  const row = rows()[0];
  const touch = { pointerType: "touch", clientX: 100, clientY: 20 };
  await fireEvent.pointerDown(row, touch);
  await waitFor(() => expect(screen.getByRole("dialog")).toBeTruthy());
  await fireEvent.pointerUp(row, touch);
  // The click that ends the long press does not follow the row's link.
  expect(await fireEvent.click(row)).toBe(false);
  await press("Escape");
  await fireEvent.pointerDown(row, touch);
  await fireEvent.pointerMove(row, { ...touch, clientY: -10 });
  await new Promise((resolve) => setTimeout(resolve, 600));
  await fireEvent.pointerUp(row, touch);
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("full-width menu and cancellable destructive confirmation", async () => {
  const { deletes, commands, hold } = plates();
  visit("/plates");
  await waitFor(() => expect(rows()).toHaveLength(2));
  await fireEvent.contextMenu(rows()[0]);
  await waitFor(() => expect(inDialog("キュー追加").disabled).toBe(false));
  const remove = inDialog("削除");
  await fireEvent.click(remove);
  expect(dialog().getAttribute("aria-label")).toBe("プレートを削除");
  expect(
    within(dialog()).getByText("「保存プレート 1」を削除しますか？"),
  ).toBeTruthy();
  const cancel = within(dialog()).getByRole("button", { name: "キャンセル" });
  await waitFor(() => expect(document.activeElement).toBe(cancel));
  expect(deletes).toHaveLength(0);
  await fireEvent.click(cancel);
  expect(dialog().getAttribute("aria-label")).toBe("保存プレート 1");
  await waitFor(() => expect(document.activeElement).toBe(inDialog("削除")));
  expect(deletes).toHaveLength(0);
  await fireEvent.click(inDialog("削除"));
  await press("Escape");
  await waitFor(() => expect(document.activeElement).toBe(inDialog("削除")));
  expect(dialog().getAttribute("aria-label")).toBe("保存プレート 1");
  expect(deletes).toHaveLength(0);
  await fireEvent.click(inDialog("削除"));
  await fireEvent.click(
    within(dialog())
      .getAllByRole("button", { name: "閉じる" })
      .find((b) => !b.classList.contains("scrim"))!,
  );
  await waitFor(() => expect(document.activeElement).toBe(inDialog("削除")));
  expect(deletes).toHaveLength(0);
  await fireEvent.click(inDialog("削除"));
  await fireEvent.click(inDialog("削除"));
  await waitFor(() => expect(rows()).toHaveLength(1));
  await waitFor(() => expect(document.activeElement).toBe(rows()[0]));
  expect(deletes).toHaveLength(1);
  expect(commands).toHaveLength(0);
  hold();
  await press("F10", rows()[0], { shiftKey: true });
  await waitFor(() => expect(inDialog("キュー追加").disabled).toBe(true));
});

it("pending deletion sends once, holds keyboard focus and failure returns to cancel", async () => {
  const { api } = plates();
  let count = 0;
  const held = gate();
  api.route(async ({ method, path }) => {
    if (method !== "DELETE" || !path.startsWith("/api/plates/")) return;
    count++;
    await held.held;
    return { status: 503, json: { error: "一時的な保存エラー" } };
  });
  visit("/plates");
  await waitFor(() => expect(rows()).toHaveLength(2));
  await fireEvent.contextMenu(rows()[0]);
  await fireEvent.click(inDialog("削除"));
  const confirm = inDialog("削除");
  confirm.click();
  confirm.click();
  await waitFor(() => expect(count).toBe(1));
  const cancel = () =>
    within(dialog()).getByRole<HTMLButtonElement>("button", {
      name: "キャンセル",
    });
  expect(cancel().disabled).toBe(true);
  for (const close of within(dialog()).getAllByRole<HTMLButtonElement>(
    "button",
    { name: "閉じる" },
  ))
    expect(close.disabled).toBe(true);
  expect(inDialog("削除中…").disabled).toBe(true);
  await press("Escape");
  expect(screen.getByRole("dialog")).toBeTruthy();
  await press("Tab");
  expect(document.activeElement).toBe(dialog());
  held.release();
  expect(await within(dialog()).findByRole("alert")).toBeTruthy();
  await waitFor(() => expect(document.activeElement).toBe(cancel()));
  await fireEvent.click(cancel());
  await waitFor(() => expect(rows()).toHaveLength(2));
  expect(count).toBe(1);
});

it("long names keep the confirmation's keyboard path", async () => {
  plates("baseplate_front_".repeat(10));
  visit("/plates");
  await waitFor(() => expect(rows()).toHaveLength(2));
  await fireEvent.contextMenu(rows()[0]);
  await fireEvent.click(inDialog("削除"));
  const cancel = within(dialog()).getByRole("button", { name: "キャンセル" });
  await waitFor(() => expect(document.activeElement).toBe(cancel));
  await fireEvent.click(cancel);
  await waitFor(() => expect(document.activeElement).toBe(inDialog("削除")));
});

it("license menu and source recovery", async () => {
  let failed = true;
  let source: string | null = "https://example.org/source.tar.gz";
  server(({ path }) => {
    if (path === "/api/plates") return { json: [] };
    if (path === "/api/about")
      return failed
        ? { status: 500, json: { error: "unavailable" } }
        : { json: { version: "0.1.8", source_url: source } };
  });
  visit("/");
  await click("メニュー");
  const link = screen.getByRole<HTMLAnchorElement>("link", {
    name: "ライセンスとソース",
  });
  expect(link.getAttribute("href")).toBe("/about");
  visit("/about");
  expect(await screen.findByRole("alert")).toBeTruthy();
  failed = false;
  await click("再試行");
  expect(
    await screen.findByRole("heading", { name: "OrcaServer 0.1.8" }),
  ).toBeTruthy();
  const href = (name: string) =>
    screen.getByRole("link", { name }).getAttribute("href");
  expect(href("このビルドのソースを取得")).toBe(source);
  expect(href("ライセンス本文")).toBe("/LICENSE");
  expect(href("第三者の著作権・許諾表示")).toBe("/THIRD_PARTY_NOTICES");
  source = null;
  reload();
  expect(await screen.findByText(/公開先が設定されていません/)).toBeTruthy();
  expect(
    screen.queryByRole("link", { name: "このビルドのソースを取得" }),
  ).toBeNull();
  expect(href("プレート一覧へ")).toBe("/plates");
  visit("/plates");
  expect(await screen.findByText("保存済みプレートはありません")).toBeTruthy();
});

const listed = Array.from({ length: 100 }, (_, i) => ({
  id: `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
  version: 1,
  name:
    i % 3 === 0
      ? `机の配線整理・ケーブルホルダー ${i + 1}`
      : `小物ケース ${i + 1}`,
  models: [{ id: "item", name: "box.stl", source: "box.stl", quantity: 1 }],
  settings: {},
  project: "project.3mf",
  print: "print.gcode.3mf",
}));
const creation: Route = ({ path }) => {
  if (path === "/api/scad/model-info") return { json: { roles: ["primary"] } };
  if (path === "/api/printers" || path === "/api/filaments")
    return { json: [] };
  if (path === "/api/default-settings")
    return {
      json: {
        default_printer_id: null,
        conditions: {
          required_machine_profile_key: null,
          filament_id: null,
          process_profile_key: null,
          bed_type: null,
        },
        reason: "printer",
      },
    };
};

it("searchable list and theme", async () => {
  server(creation, ({ path, url }) => {
    if (path !== "/api/plates") return;
    const q = url.searchParams.get("q");
    return { json: q ? listed.filter((p) => p.name.includes(q)) : listed };
  });
  visit("/plates");
  await waitFor(() => expect(rows()).toHaveLength(100));
  expect(document.title).toBe("OrcaServer");
  expect(document.querySelectorAll("header a, header button")).toHaveLength(3);
  const search = screen.getByRole("searchbox");
  search.focus();
  await press("ArrowDown");
  expect(document.activeElement).toBe(rows()[0]);
  await press("ArrowDown");
  expect(document.activeElement).toBe(rows()[1]);
  await fireEvent.input(search, { target: { value: "小物ケース 20" } });
  await waitFor(() => expect(rows()).toHaveLength(1));
  await fireEvent.input(search, { target: { value: "" } });
  await waitFor(() => expect(rows()).toHaveLength(100));
  const menu = screen.getByRole("button", { name: "メニュー" });
  menu.focus();
  await fireEvent.click(menu);
  await click("テーマ設定");
  const theme = await screen.findByRole("dialog", { name: "テーマ設定" });
  await fireEvent.click(screen.getByRole("radio", { name: "ライト" }));
  expect(document.documentElement.dataset.theme).toBe("light");
  expect(screen.getByRole("dialog", { name: "テーマ設定" })).toBe(theme);
  await press("Escape");
  await waitFor(() =>
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: "メニュー" }),
    ),
  );
  reload();
  expect(document.documentElement.dataset.theme).toBe("light");
  await click("メニュー");
  await click("テーマ設定");
  await fireEvent.click(screen.getByRole("radio", { name: "自動" }));
  expect(localStorage.getItem("orca-server:theme")).toBeNull();
  expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  await press("Escape");
  await waitFor(() => expect(rows()).toHaveLength(100));
});

it("loading and failed list recover through retry", async () => {
  const finish = gate();
  let fail = true;
  server(async ({ path }) => {
    if (path !== "/api/plates") return;
    if (!fail) return { json: [] };
    await finish.held;
    return { status: 500, json: { error: "unavailable" } };
  });
  visit("/plates");
  expect((await screen.findByRole("status")).textContent).toContain(
    "読み込んでいます",
  );
  finish.release();
  expect((await screen.findByRole("alert")).textContent).toContain(
    "処理に失敗しました",
  );
  fail = false;
  await click("再試行");
  expect(await screen.findByText("保存済みプレートはありません")).toBeTruthy();
});

it("keyboard selection persists across filtering and failed saves retain edits", async () => {
  const bodies: any[] = [];
  server(creation, ({ path, url, body }) => {
    if (path === "/api/scad/models") {
      const q = url.searchParams.get("q") || "";
      return { json: ["box.stl", "holder.stl"].filter((n) => n.includes(q)) };
    }
    if (path === "/api/plates/import") {
      bodies.push(body);
      return { status: 400, json: { error: "bad composition" } };
    }
  });
  visit("/plates/new");
  await waitFor(() => expect(screen.getAllByRole("checkbox")).toHaveLength(2));
  const search = screen.getByRole("searchbox");
  search.focus();
  await press("ArrowDown");
  const box = screen.getByRole<HTMLInputElement>("checkbox", {
    name: "box.stl",
  });
  expect(document.activeElement).toBe(box);
  // Space on a focused checkbox is the browser's own toggle.
  await fireEvent.click(box);
  expect(box.checked).toBe(true);
  await fireEvent.input(search, { target: { value: "holder" } });
  await waitFor(() => expect(screen.getAllByRole("checkbox")).toHaveLength(1));
  await fireEvent.click(screen.getByRole("checkbox"));
  await click("構成を確認（2）");
  await fireEvent.input(screen.getByLabelText("プレート名"), {
    target: { value: "小物入れ" },
  });
  await fireEvent.input(screen.getByLabelText("box.stl の個数"), {
    target: { value: "3" },
  });
  await click("保存");
  await waitFor(() =>
    expect(
      screen
        .getAllByRole("alert")
        .some((a) => a.textContent?.includes("入力やモデルを確認してください")),
    ).toBe(true),
  );
  await click("モデル選択へ");
  await fireEvent.click(
    await screen.findByRole("checkbox", { name: "holder.stl" }),
  );
  await click("構成を確認（1）");
  expect(screen.getByLabelText<HTMLInputElement>("プレート名").value).toBe(
    "小物入れ",
  );
  expect(screen.getByLabelText<HTMLInputElement>("box.stl の個数").value).toBe(
    "3",
  );
  await click("保存");
  await waitFor(() => expect(bodies).toHaveLength(2));
  expect(bodies[1].models).toEqual([
    { name: "box.stl", source: "box.stl", quantity: 3 },
  ]);
});

it("unconfigured services expose recovery without hiding the plate list", async () => {
  server(({ path }) => {
    if (path === "/api/slicer/profiles" || path === "/api/scad/models")
      return { status: 503, json: { error: "unset" } };
  });
  visit("/plates/new");
  await waitFor(() => expect(screen.getAllByRole("alert")).toHaveLength(1));
  expect(
    screen.getByRole<HTMLButtonElement>("button", { name: "構成を確認（0）" })
      .disabled,
  ).toBe(true);
  expect(screen.getByRole("button", { name: "再試行" })).toBeTruthy();
  expect(
    screen.getByRole("link", { name: "プレート一覧へ" }).getAttribute("href"),
  ).toBe("/plates");
});
