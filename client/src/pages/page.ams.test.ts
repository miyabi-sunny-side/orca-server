import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, press, reset, visit } from "../testing/page";
import { gate, server } from "../testing/server";

afterEach(reset);

const white = {
  id: "white",
  name: "PLA 白",
  vendor: "Fixture",
  material: "PLA",
  color: "FFFFFFFF",
};
const slot = (index: number, change: object = {}) => ({
  id: `slot-${index}`,
  ams_id: 0,
  slot_index: index,
  filament_id: index === 0 ? "white" : null,
  mapping_source: index === 0 ? "manual" : "none",
  revision: 1,
  load_order: index,
  priority_order: index,
  priority_group: [],
  backup_peers: [],
  current: true,
  detect_on_insert: true,
  detect_on_power_up: true,
  filament: index === 0 ? white : null,
  setting:
    index === 0
      ? {
          resolved: {
            nozzle_temperature_initial_layer: 220,
            nozzle_temperature: 220,
          },
        }
      : null,
  reported: {
    present: index === 0,
    material: index === 0 ? "PLA" : null,
    brand: null,
    color: index === 0 ? "FFFFFFFF" : null,
    profile_id: null,
    tag_uid: null,
    temperature_min: 190,
    temperature_max: 230,
    remaining_percent: null,
    last_seen_at: 1750000000,
  },
  ...change,
});
/** One AMS of four slots: the first holds PLA 白, the rest are empty. */
function ams() {
  const inventory: any = {
    printer_id: "p1",
    current: true,
    slots: [0, 1, 2, 3].map((i) => slot(i)),
    auto_refill: { supported: false, enabled: false, groups: null },
  };
  const api = server(({ method, path, body }) => {
    if (path === "/api/printers/p1")
      return {
        json: {
          id: "p1",
          name: "P1S",
          machine_profile_key: "Bambu Lab P1S 0.4 nozzle",
        },
      };
    if (path === "/api/printers/p1/ams")
      return { json: structuredClone(inventory) };
    if (path === "/api/filaments") return { json: [white] };
    if (method === "PUT" && path.startsWith("/api/printers/p1/ams/")) {
      const target = inventory.slots.find((s: any) =>
        path.endsWith(`/${s.id}`),
      );
      if (target) {
        target.filament_id = body.filament_id;
        target.filament = body.filament_id ? white : null;
        target.revision++;
      }
      return { status: 204 };
    }
  });
  return { api, inventory };
}
const row = (n: number) =>
  screen
    .getByRole("button", { name: `AMS 0 スロット ${n}の詳細` })
    .closest("li")!;
const material = (n: number) =>
  within(row(n)).getByRole<HTMLButtonElement>("button", { name: /材料を選択/ });
/** Opens the material search of slot `n` once nothing else is being saved. */
async function searchIn(n: number) {
  await waitFor(() => expect(material(n).disabled).toBe(false));
  await fireEvent.click(material(n));
  return within(row(n)).findByRole<HTMLInputElement>("searchbox", {
    name: "材料を検索",
  });
}
/** Long enough for the page's five-second refresh. */
const polled = { timeout: 7000 };

it("compact AMS selection preserves editing during polling and changes start priority", async () => {
  const { api, inventory } = ams();
  inventory.slots[0].priority_group = [
    { id: "slot-2", revision: 1 },
    { id: "slot-0", revision: 1 },
  ];
  visit("/printers/p1/ams");
  await waitFor(() => expect(material(1).textContent).toContain("PLA 白"));
  expect(screen.queryByText(/設定温度:/)).toBeNull();
  await fireEvent.click(
    within(row(1)).getByRole("button", { name: "AMS 0 スロット 1の詳細" }),
  );
  expect(within(row(1)).getByText(/設定温度:/)).toBeTruthy();
  await fireEvent.change(within(row(1)).getByLabelText("印刷開始時の使用順"), {
    target: { value: "0" },
  });
  await waitFor(() =>
    expect(api.to("PUT", "/api/printers/p1/ams/priority")).toHaveLength(1),
  );
  expect(
    api.to("PUT", "/api/printers/p1/ams/priority")[0].body.order[0].id,
  ).toBe("slot-0");
  await fireEvent.click(material(1));
  const search = await within(row(1)).findByRole<HTMLInputElement>(
    "searchbox",
    {
      name: "材料を検索",
    },
  );
  await fireEvent.input(search, { target: { value: "Fixture" } });
  expect(
    await within(row(1)).findByRole("button", { name: /PLA 白.*Fixture/ }),
  ).toBeTruthy();
  // The five-second refresh keeps the search, its focus and the open details.
  search.focus();
  const reads = api.to("GET", "/api/printers/p1/ams").length;
  await waitFor(
    () =>
      expect(api.to("GET", "/api/printers/p1/ams").length).toBeGreaterThan(
        reads,
      ),
    polled,
  );
  expect(search.value).toBe("Fixture");
  expect(document.activeElement).toBe(search);
  expect(within(row(1)).getByText(/設定温度:/)).toBeTruthy();
  await press("ArrowDown", search);
  const clear = within(row(1)).getByRole("button", { name: "指定を解除" });
  expect(document.activeElement).toBe(clear);
  await fireEvent.click(clear);
  await waitFor(() =>
    expect(material(1).textContent).toContain("材料は未指定"),
  );
  expect(api.to("PUT", "/api/printers/p1/ams/slot-0")[0].body).toEqual({
    revision: 1,
    filament_id: null,
  });
  await fireEvent.input(await searchIn(1), { target: { value: "白" } });
  await fireEvent.click(
    await within(row(1)).findByRole("button", { name: /PLA 白.*Fixture/ }),
  );
  await waitFor(() => expect(material(1).textContent).toContain("PLA 白"));
  // A new observation of the slot while choosing asks to choose again.
  await fireEvent.input(await searchIn(1), { target: { value: "Fixture" } });
  inventory.slots[0].revision++;
  inventory.slots[0].reported.tag_uid = "replacement";
  expect(
    (await within(row(1)).findByRole("alert", {}, polled)).textContent,
  ).toContain("観測情報が変わりました");
  expect(
    within(row(1)).getByRole<HTMLButtonElement>("button", {
      name: /PLA 白.*Fixture/,
    }).disabled,
  ).toBe(true);
  await click("選び直す", row(1));
  await waitFor(() =>
    expect(
      within(row(1)).getByRole<HTMLButtonElement>("button", {
        name: /PLA 白.*Fixture/,
      }).disabled,
    ).toBe(false),
  );
  await fireEvent.click(
    within(row(1)).getByRole("button", { name: /PLA 白.*Fixture/ }),
  );
  await waitFor(() =>
    expect(
      api.to("PUT", "/api/printers/p1/ams/slot-0").at(-1)!.body.revision,
    ).toBe(inventory.slots[0].revision - 1),
  );
  // Auto refill follows what the printer reports.
  const refill = () =>
    [...document.querySelectorAll("details")].find(
      (d) => d.querySelector("summary")?.textContent?.trim() === "自動補充",
    )!;
  await fireEvent.click(refill().querySelector("summary")!);
  expect(
    within(refill()).getByText("プリンターが非対応と報告しています。"),
  ).toBeTruthy();
  inventory.auto_refill = { supported: true, enabled: false, groups: null };
  await waitFor(
    () =>
      expect(
        within(refill()).getByRole("button", { name: "有効にする" }),
      ).toBeTruthy(),
    polled,
  );
  await click("有効にする", refill());
  await waitFor(() =>
    expect(api.to("PUT", "/api/printers/p1/ams/auto-refill")[0]?.body).toEqual({
      enabled: true,
    }),
  );
  await waitFor(() =>
    expect(within(refill()).getByRole("status").textContent).toContain(
      "設定を送信しました",
    ),
  );
  inventory.auto_refill.enabled = true;
  await waitFor(
    () =>
      expect(screen.getByLabelText("自動補充の設定状態").textContent).toContain(
        "有効",
      ),
    polled,
  );
}, 30_000);

it("状態を更新 asks the printer, waits for its report and keeps the page state", async () => {
  const { api, inventory } = ams();
  const asked = gate();
  let answer: () => { status?: number; json?: unknown } = () => ({
    json: {
      ...structuredClone(inventory),
      slots: inventory.slots.map((s: any, i: number) =>
        i === 1
          ? {
              ...s,
              reported: { ...s.reported, present: true, color: "FFFFFFFF" },
            }
          : s,
      ),
      refresh: { report_at: 1750000000, reading: false },
    },
  });
  let held = true;
  api.route(async ({ method, path }) => {
    if (path !== "/api/printers/p1/ams/refresh" || method !== "POST") return;
    if (held) await asked.held;
    return answer();
  });
  visit("/printers/p1/ams");
  await waitFor(() => expect(material(2).textContent).toContain("空"));
  // An open detail and a search in progress stay across the re-read.
  await fireEvent.click(
    within(row(1)).getByRole("button", { name: "AMS 0 スロット 1の詳細" }),
  );
  await fireEvent.click(material(1));
  const search = await within(row(1)).findByRole<HTMLInputElement>(
    "searchbox",
    {
      name: "材料を検索",
    },
  );
  await fireEvent.input(search, { target: { value: "Fixture" } });
  const refresh = () =>
    screen.getByRole<HTMLButtonElement>("button", { name: "状態を更新" });
  const says = (text: string) =>
    waitFor(() =>
      expect(
        screen
          .getAllByRole("status")
          .some((s) => s.textContent?.includes(text)),
      ).toBe(true),
    );
  await fireEvent.click(refresh());
  await says("本体から取得しています");
  expect(refresh().disabled).toBe(true);
  asked.release();
  await says("プリンターの最新状態を反映しました");
  expect(refresh().disabled).toBe(false);
  expect(material(2).textContent).toContain("材料は未指定");
  await fireEvent.click(
    within(row(2)).getByRole("button", { name: "AMS 0 スロット 2の詳細" }),
  );
  expect(within(row(2)).getByText(/装填あり/)).toBeTruthy();
  expect(within(row(2)).getByText("#FFFFFFFF")).toBeTruthy();
  expect(search.value).toBe("Fixture");
  expect(within(row(1)).getByText(/設定温度:/)).toBeTruthy();
  held = false;
  // Reading in progress is not presented as a finished read.
  const reading = answer;
  answer = () => {
    const view = reading() as { json: any };
    view.json.refresh.reading = true;
    return view;
  };
  await click("状態を更新");
  await says("AMSはまだ材料を読み取り中です");
  // A lost connection is an alert with its cause; the same button retries.
  answer = () => ({ status: 502, json: { error: "Printer is not connected" } });
  await click("状態を更新");
  expect((await screen.findByRole("alert")).textContent).toContain(
    "プリンターに接続できないため",
  );
  expect(refresh().disabled).toBe(false);
  answer = reading;
  await click("状態を更新");
  await says("プリンターの最新状態を反映しました");
  expect(screen.queryByRole("alert")).toBeNull();
});
