import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeAll, expect, it } from "vitest";
import { click, press, reload, reset, visit } from "../testing/page";
import { queueFixture } from "../testing/queue";

beforeAll(() => {
  process.env.TZ = "Asia/Tokyo";
});
afterEach(reset);

/** Fifty finished prints of the current plate; the third plate is gone. */
function history() {
  const f = queueFixture();
  const plate = {
    id: f.q.current.plate_id,
    name: "現在の名前",
    version: 7,
    conditions: { filament_id: null },
    models: [],
  };
  const items = Array.from({ length: 50 }, (_, i) => ({
    id: 100 - i,
    name:
      i === 0
        ? "長いプレート名・前面の取り付け部品と追加のケース"
        : `部品 ${i}`,
    plate_id: plate.id,
    printer_id: "p1",
    completed_at: 1750000000 - i,
    available: i !== 2,
  }));
  let fail = false,
    empty = false,
    hold = false;
  f.api.route(({ method, path, url, body }) => {
    if (path === "/api/history") {
      if (fail)
        return { status: 503, json: { error: "一時的に取得できません" } };
      const before = url.searchParams.has("before");
      return {
        json: {
          items: empty
            ? []
            : before
              ? [{ ...items[0], id: 1, name: "以前の印刷" }]
              : items,
          next_cursor: empty || before ? null : "1749999951:51",
        },
      };
    }
    if (path.startsWith("/api/plates/")) return { json: plate };
    if (path === "/api/queue" && url.search) {
      if (method === "POST") {
        f.commands.push(body);
        f.q.generation++;
      }
      return {
        json: {
          ...structuredClone(f.q),
          request_id: `request-${f.q.generation}`,
          admission: {
            allowed: !hold,
            plate_version: 7,
            reason: hold
              ? "No confirmed AMS slot contains the plate material"
              : null,
          },
        },
      };
    }
  });
  return {
    ...f,
    items,
    fail: (v: boolean) => (fail = v),
    empty: () => (empty = true),
    hold: (v: boolean) => (hold = v),
  };
}
const rows = () => [...document.querySelectorAll<HTMLElement>(".history-row")];
const dialog = () => screen.getByRole("dialog");
const addButton = () =>
  within(dialog()).getByRole<HTMLButtonElement>("button", {
    name: "キューに追加",
  });

it("history menu, dates, paging and focus", async () => {
  const f = history();
  visit("/");
  expect(
    (await screen.findByRole("link", { name: "プリント履歴" })).getAttribute(
      "href",
    ),
  ).toBe("/history");
  visit("/history");
  await waitFor(() => expect(rows()).toHaveLength(50));
  expect(rows()[0].textContent).toContain("2025/06/16 00:06:40");
  expect(rows()[0].querySelector("time")!.getAttribute("datetime")).toBe(
    "2025-06-15T15:06:40.000Z",
  );
  await fireEvent.contextMenu(rows()[0]);
  await waitFor(() => expect(addButton().disabled).toBe(false));
  await press("Escape");
  await waitFor(() => expect(document.activeElement).toBe(rows()[0]));
  await press("F10", rows()[0], { shiftKey: true });
  await click("キューに追加", dialog());
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(rows()[0]));
  expect(f.commands).toHaveLength(1);
  expect(f.commands[0].action).toEqual({
    type: "add",
    plate_id: f.items[0].plate_id,
    plate_version: 7,
  });
  await waitFor(() =>
    expect(
      screen
        .getAllByRole("status")
        .some((s) => s.textContent?.includes("キューに追加しました")),
    ).toBe(true),
  );
  expect(
    screen.getByRole("link", { name: "キューを見る" }).getAttribute("href"),
  ).toBe("/queue?printer_id=p1");
  await press("ContextMenu", rows()[2]);
  await waitFor(() => expect(addButton().disabled).toBe(true));
  expect(dialog().textContent).toContain(
    "プレートが削除されているため追加できません",
  );
  await fireEvent.click(document.querySelector(".scrim")!);
  await waitFor(() => expect(document.activeElement).toBe(rows()[2]));
  await click("以前の履歴を読み込む");
  await waitFor(() => expect(rows()).toHaveLength(51));
  expect(rows().at(-1)!.textContent).toContain("以前の印刷");
});

it("load errors retry without discarding earlier pages; empty and unavailable conditions", async () => {
  const f = history();
  f.fail(true);
  visit("/history");
  expect((await screen.findByRole("alert")).textContent).toContain(
    "履歴を取得できませんでした",
  );
  f.fail(false);
  await click("再試行");
  await waitFor(() => expect(rows()).toHaveLength(50));
  f.fail(true);
  await click("以前の履歴を読み込む");
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(rows()).toHaveLength(50);
  f.fail(false);
  await click("再試行");
  await waitFor(() => expect(rows()).toHaveLength(51));
  f.hold(true);
  await fireEvent.contextMenu(rows()[0]);
  await waitFor(() => expect(addButton().disabled).toBe(true));
  expect(await screen.findByRole("link", { name: "AMSを確認" })).toBeTruthy();
  expect(
    await screen.findByRole("link", { name: "プレート編集" }),
  ).toBeTruthy();
  await press("Escape");
  f.empty();
  reload();
  expect(await screen.findByText("プリント履歴はまだありません")).toBeTruthy();
});

it("original printer wins over saved selection; unavailable references remain readable", async () => {
  const f = history();
  const machine = "Bambu Lab P1S 0.4 nozzle";
  let original = true;
  let deleted = false;
  f.api.route(({ path }) => {
    if (path === "/api/printers")
      return {
        json: [
          ...(original
            ? [{ id: "p1", name: "Original", machine_profile_key: machine }]
            : []),
          { id: "p2", name: "Alternative", machine_profile_key: machine },
        ],
      };
    if (deleted && path.startsWith("/api/plates/"))
      return { status: 404, json: { error: "deleted" } };
  });
  localStorage.setItem(`orca-plate-printer:${f.items[0].plate_id}`, "p2");
  visit("/history");
  await waitFor(() => expect(rows()).toHaveLength(50));
  const row = rows()[0];
  await fireEvent.contextMenu(row);
  await waitFor(async () =>
    expect(
      (await screen.findByLabelText<HTMLSelectElement>("追加先のプリンター"))
        .value,
    ).toBe("p1"),
  );
  await press("Escape");
  original = false;
  await fireEvent.contextMenu(row);
  await waitFor(() =>
    expect(dialog().textContent).toContain("追加先: Alternative"),
  );
  await waitFor(() => expect(addButton().disabled).toBe(false));
  await press("Escape");
  deleted = true;
  await fireEvent.contextMenu(row);
  await waitFor(() =>
    expect(dialog().textContent).toContain(
      "プレートが削除されているため追加できません",
    ),
  );
  expect(addButton().disabled).toBe(true);
  await press("Escape");
  expect(row.textContent).toContain(f.items[0].name);
});
