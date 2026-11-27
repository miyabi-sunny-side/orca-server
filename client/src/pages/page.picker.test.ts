import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, press, reset, visit, went } from "../testing/page";
import { gate, server } from "../testing/server";

afterEach(reset);

const id = "11111111-1111-4111-8111-111111111111";
const all = "所持していないフィラメントを選択する";
const filament = (id: string, name: string, material = "PLA") => ({
  id,
  name,
  vendor: "Fixture",
  material,
  color: "FFFFFF",
});
const white = filament("white", "PLA 白"),
  blue = filament("blue", "PLA 青"),
  mini = filament("mini", "PLA 黒 A1 mini"),
  other = filament("other", "PLA 灰"),
  future = filament("future", "PETG-GF 長い名前の材料", "PETG-GF");

/**
 * The materials every printer has loaded (four), one more that none has, and
 * what each printer reports: `p1` may lose the mapping of a slot, stop
 * confirming its AMS, or report it empty.
 */
function picker() {
  const state = {
    p1: "current" as "current" | "unconfirmed",
    unassigned: false,
    empty: false,
    missing: false,
  };
  let plate: any = {
    id,
    version: 1,
    name: "材料を選ぶプレート",
    models: [{ id: "m", name: "cube.stl", source: "cube.stl", quantity: 1 }],
    conditions: { filament_id: "white" },
  };
  const api = server(({ method, path, url, body }) => {
    if (path === "/api/default-settings")
      return {
        json: {
          default_printer_id: "p1",
          conditions: { filament_id: "white" },
          infill_patterns: ["adaptivecubic"],
          reason: null,
        },
      };
    if (path === "/api/printers")
      return {
        json: [
          {
            id: "p1",
            name: "P1S",
            machine_profile_key: "Bambu Lab P1S 0.4 nozzle",
          },
        ],
      };
    if (path === "/api/scad/model-info")
      return { json: { roles: ["primary"] } };
    if (path === "/api/scad/models") return { json: ["cube.stl"] };
    if (path === `/api/plates/${id}`) {
      if (method === "PUT")
        plate = { ...plate, ...body, version: plate.version + 1 };
      return { json: plate };
    }
    if (path === "/api/plates/import")
      return {
        status: 201,
        json: {
          id: "22222222-2222-4222-8222-222222222222",
          version: 1,
          ...body,
        },
      };
    if (path === "/api/plate-filaments") {
      const q = (url.searchParams.get("q") ?? "").toLowerCase();
      const unloaded = url.searchParams.get("include_unloaded") === "true";
      const chosen = url.searchParams.get("selected_id");
      const loaded =
        state.p1 !== "current" || state.empty
          ? []
          : [white, ...(state.unassigned ? [] : [blue]), mini, other];
      const known = [white, blue, mini, other, future];
      const found = (unloaded ? known : loaded).filter((f) =>
        `${f.name} ${f.vendor}`.toLowerCase().includes(q),
      );
      const selected =
        state.missing && chosen === "future"
          ? null
          : (known.find((f) => f.id === chosen) ?? null);
      return {
        json: {
          filaments: found,
          selected,
          selected_state: !chosen
            ? "unset"
            : !selected
              ? "missing"
              : state.p1 === "unconfirmed"
                ? "unconfirmed"
                : loaded.some((f) => f.id === chosen)
                  ? "loaded"
                  : "unloaded",
          loaded_ids: loaded.map((f) => f.id),
          printers: [
            {
              id: "p1",
              name: "P1S",
              state: state.p1,
              unassigned: state.unassigned,
            },
            { id: "a1", name: "A1 mini", state: state.p1, unassigned: false },
          ],
        },
      };
    }
  });
  return { api, state, plate: () => plate };
}

const main = () =>
  screen.getByRole<HTMLButtonElement>("button", { name: /^フィラメント:/ });
const support = () =>
  screen.getByRole<HTMLButtonElement>("button", {
    name: /^接触面のフィラメント:/,
  });
const dialog = () => screen.getByRole("dialog");
const search = () =>
  within(dialog()).getByRole<HTMLInputElement>("searchbox", {
    name: "材料を検索",
  });
const choices = () => [
  ...dialog().querySelectorAll<HTMLButtonElement>(".choices button"),
];
const option = (id: string) =>
  dialog().querySelector<HTMLButtonElement>(`button[data-filament-id="${id}"]`);
const scope = () => within(dialog()).getByLabelText<HTMLInputElement>(all);
const type = (value: string) =>
  fireEvent.input(search(), { target: { value } });
const closed = () =>
  waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
const focused = (element: () => Element) =>
  waitFor(() => expect(document.activeElement).toBe(element()));
async function openMain() {
  await fireEvent.click(main());
  await focused(search);
}
async function edit() {
  visit(`/plates/${id}`);
  await click("構成を編集");
  await waitFor(() => expect(main()).toBeTruthy());
}

it("loaded/all selection, keyboard and persistence", async () => {
  const { api } = picker();
  visit("/plates/new");
  await fireEvent.click(await screen.findByRole("checkbox"));
  await click("構成を確認（1）");
  await waitFor(() => expect(main().textContent).toContain("PLA 白"));
  await fireEvent.input(screen.getByLabelText("プレート名"), {
    target: { value: "Picker" },
  });
  await openMain();
  expect(scope().checked).toBe(false);
  await waitFor(() => expect(choices()).toHaveLength(4));
  // Plates name no machine: every printer's loaded materials are candidates.
  expect(option("mini")).toBeTruthy();
  expect(option("future")).toBeNull();
  await type("Fixture");
  await waitFor(() => expect(choices()).toHaveLength(4));
  await fireEvent.click(scope());
  expect(search().value).toBe("Fixture");
  await waitFor(() => expect(choices()).toHaveLength(5));
  await fireEvent.click(scope());
  await waitFor(() => expect(choices()).toHaveLength(4));
  // Each closing path restores the initiating control.
  search().focus();
  await press("Escape", search());
  await focused(main);
  expect(main().textContent).toContain("PLA 白");
  for (const way of ["キャンセル", "閉じる", "scrim"]) {
    await fireEvent.click(main());
    const button =
      way === "scrim"
        ? document.querySelector<HTMLButtonElement>(".scrim")!
        : within(dialog())
            .getAllByRole("button", { name: way })
            .find((b) => !b.classList.contains("scrim"))!;
    await fireEvent.click(button);
    await focused(main);
  }
  await openMain();
  await fireEvent.click(scope());
  await type("GF");
  await waitFor(() => expect(option("future")).toBeTruthy());
  await press("ArrowDown", search());
  expect(document.activeElement).toBe(option("future"));
  await fireEvent.click(option("future")!);
  await focused(main);
  await waitFor(() => expect(main().textContent).toContain("未装填"));
  await openMain();
  expect(scope().checked).toBe(false);
  await waitFor(() => expect(choices()).toHaveLength(4));
  await press("Escape", search());
  await fireEvent.click(
    [...document.querySelectorAll("summary")].find(
      (s) => s.textContent?.trim() === "詳細設定",
    )!,
  );
  await fireEvent.click(screen.getByLabelText("サポートを使う"));
  await waitFor(() => expect(support().textContent).toContain("PETG-GF"));
  await fireEvent.click(support());
  await focused(search);
  await waitFor(() => expect(choices()).toHaveLength(4));
  await type("PLA 青");
  await waitFor(() => expect(option("blue")).toBeTruthy());
  await fireEvent.click(option("blue")!);
  await focused(support);
  await click("保存");
  await waitFor(() => expect(went).toHaveLength(1));
  const saved = api.to("POST", "/api/plates/import")[0].body.conditions;
  expect(saved).toMatchObject({
    filament_id: "future",
    support_interface_filament_id: "blue",
    support_enabled: true,
  });
  expect(saved).not.toHaveProperty("include_unloaded");
});

it("the support material can follow the main material again", async () => {
  const { api, plate } = picker();
  Object.assign(plate().conditions, {
    filament_id: "future",
    support_enabled: true,
    support_interface_filament_id: "blue",
  });
  await edit();
  await waitFor(() => expect(main().textContent).toContain("未装填"));
  await fireEvent.click(
    [...document.querySelectorAll("summary")].find(
      (s) => s.textContent?.trim() === "詳細設定",
    )!,
  );
  await waitFor(() => expect(support().textContent).toContain("PLA 青"));
  await fireEvent.click(support());
  await click("主材料と同じにする", dialog());
  await waitFor(() => expect(support().textContent).toContain("PETG-GF"));
  await focused(support);
  await click("保存");
  await waitFor(() =>
    expect(api.to("PUT", `/api/plates/${id}`)).toHaveLength(1),
  );
  // No own material: the server takes the main one for the support.
  expect(
    api.to("PUT", `/api/plates/${id}`)[0].body.conditions
      .support_interface_filament_id,
  ).toBeNull();
});

it("late and failed responses retain query, scope and selection", async () => {
  const { api } = picker();
  const late = gate();
  let failing = true;
  api.route(async ({ path, url }) => {
    if (path !== "/api/plate-filaments") return;
    const q = url.searchParams.get("q");
    if (q === "GF") await late.held;
    if (q === "lost" && failing)
      return { status: 503, json: { error: "Fixture unavailable" } };
  });
  await edit();
  await openMain();
  await fireEvent.click(scope());
  await type("GF");
  await type("PLA 青");
  await waitFor(() => expect(option("blue")).toBeTruthy());
  late.release();
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(search().value).toBe("PLA 青");
  expect(option("future")).toBeNull();
  await type("lost");
  expect(await within(dialog()).findByRole("alert")).toBeTruthy();
  expect(search().value).toBe("lost");
  expect(scope().checked).toBe(true);
  failing = false;
  await click("検索をやり直す", dialog());
  await waitFor(() => expect(within(dialog()).queryByRole("alert")).toBeNull());
  expect(search().value).toBe("lost");
  expect(
    await within(dialog()).findByText(/一致する材料がありません。/),
  ).toBeTruthy();
  await press("Escape", search());
  await closed();
  expect(main().textContent).toContain("PLA 白");
  // Plates name no machine: the candidates are every printer's loaded materials.
  expect(screen.queryByLabelText("要求する機種・ノズル")).toBeNull();
  await openMain();
  await waitFor(() => expect(choices()).toHaveLength(4));
  expect(option("mini")).toBeTruthy();
  await fireEvent.click(option("blue")!);
  await waitFor(() => expect(main().textContent).toContain("PLA 青"));
});

it("unknown, unmapped, empty and missing references remain distinct; clear saves null", async () => {
  const { api, state, plate } = picker();
  // Removing a mapping leaves physical material present, and offers the AMS correction path.
  state.unassigned = true;
  await edit();
  await openMain();
  const unmapped = await within(dialog()).findByText(/^P1S: 未割当の材料/);
  expect(
    within(unmapped)
      .getByRole("link", { name: "AMSの材料割当" })
      .getAttribute("href"),
  ).toBe("/printers/p1/ams");
  expect(option("blue")).toBeNull();
  await type("PLA");
  state.p1 = "unconfirmed";
  await fireEvent.click(scope());
  await fireEvent.click(scope());
  expect(search().value).toBe("PLA");
  await waitFor(() => expect(choices()).toHaveLength(0));
  expect(
    await within(dialog()).findByRole("button", { name: "装填情報を再確認" }),
  ).toBeTruthy();
  expect(scope().checked).toBe(false);
  await press("Escape", search());
  await waitFor(() => expect(main().textContent).toContain("装填未確認"));
  // An explicit empty report is unloaded, not unknown.
  Object.assign(state, { p1: "current", unassigned: false, empty: true });
  await openMain();
  await waitFor(() => expect(choices()).toHaveLength(0));
  expect(
    within(dialog()).queryByRole("button", { name: "装填情報を再確認" }),
  ).toBeNull();
  await press("Escape", search());
  await waitFor(() => expect(main().textContent).toContain("未装填"));
  await openMain();
  await click("指定を解除", dialog());
  await waitFor(() =>
    expect(main().textContent).toContain("フィラメントを選択"),
  );
  await click("保存");
  await waitFor(() =>
    expect(api.to("PUT", `/api/plates/${id}`)).toHaveLength(1),
  );
  expect(plate().conditions.filament_id).toBeNull();
  await edit();
  expect(main().textContent).toContain("フィラメントを選択");
  await openMain();
  await fireEvent.click(scope());
  await waitFor(() => expect(option("future")).toBeTruthy());
  await fireEvent.click(option("future")!);
  await waitFor(() => expect(main().textContent).toContain("未装填"));
  await click("保存");
  await waitFor(() =>
    expect(api.to("PUT", `/api/plates/${id}`)).toHaveLength(2),
  );
  expect(plate().conditions.filament_id).toBe("future");
  // A material whose registration is gone says so and is not offered.
  state.missing = true;
  await edit();
  await waitFor(() =>
    expect(main().textContent).toContain("登録が見つかりません"),
  );
  expect(screen.queryByRole("link", { name: "材料設定へ" })).toBeNull();
});

it("role materials save, reopen and are asked for with the keyboard", async () => {
  const { api } = picker();
  api.route(({ path, url }) => {
    if (path === "/api/scad/models")
      return { json: ["parts/roles.3mf", "parts/cube.stl"] };
    if (path === "/api/scad/model-info")
      return {
        json: {
          roles: url.searchParams.get("path")?.endsWith("roles.3mf")
            ? ["primary", "secondary"]
            : ["primary"],
        },
      };
  });
  visit("/plates/new");
  await fireEvent.click(
    await screen.findByRole("checkbox", { name: "parts/roles.3mf" }),
  );
  await click("構成を確認（1）");
  await fireEvent.input(screen.getByLabelText("プレート名"), {
    target: { value: "役割材料" },
  });
  const role = (name: string) =>
    screen.getByRole<HTMLButtonElement>("button", {
      name: new RegExp(`^${name}のフィラメント:`),
    });
  await waitFor(() => expect(role("primary").textContent).toContain("PLA 白"));
  expect(role("secondary").textContent).toContain("フィラメントを選択");
  await fireEvent.click(role("secondary"));
  expect(
    screen.getByRole("dialog", { name: "secondaryのフィラメントを選択" }),
  ).toBeTruthy();
  await focused(search);
  await type("PLA 青");
  await waitFor(() => expect(choices()).toHaveLength(1));
  await press("ArrowDown", search());
  expect(document.activeElement).toBe(option("blue"));
  await fireEvent.click(option("blue")!);
  await focused(() => role("secondary"));
  expect(role("secondary").textContent).toContain("PLA 青");
  await fireEvent.click(role("primary"));
  await press("Escape", search());
  await focused(() => role("primary"));
  await click("保存");
  await waitFor(() => expect(went).toHaveLength(1));
  expect(api.to("POST", "/api/plates/import")[0].body.conditions).toMatchObject(
    {
      filament_id: "white",
      secondary_filament_id: "blue",
    },
  );
  // A model with one role asks for one material only.
  visit("/plates/new");
  await fireEvent.click(
    await screen.findByRole("checkbox", { name: "parts/cube.stl" }),
  );
  await click("構成を確認（1）");
  await waitFor(() => expect(main()).toBeTruthy());
  expect(
    screen.queryByRole("button", { name: /secondaryのフィラメント/ }),
  ).toBeNull();
});
