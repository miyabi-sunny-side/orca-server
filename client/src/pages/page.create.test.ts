import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, press, reset, visit, went } from "../testing/page";
import { gate, server } from "../testing/server";

afterEach(reset);

const id = "11111111-1111-4111-8111-111111111111";
const emptyDefaults = {
  default_printer_id: null,
  conditions: { filament_id: null },
  reason: "printer",
};

it("a plate saves model references and quantities with nullable print conditions", async () => {
  const plate = {
    id,
    version: 1,
    name: "机の箱",
    models: [
      {
        id: "item-reference",
        name: "parts/box.stl",
        source: "parts/box.stl",
        quantity: 3,
      },
    ],
  };
  const api = server(({ path }) => {
    if (path === "/api/default-settings") return { json: emptyDefaults };
    if (path === "/api/scad/model-info")
      return { json: { roles: ["primary"] } };
    if (path === "/api/scad/models")
      return { json: ["parts/box.stl", "parts/lid.stl"] };
    if (path === "/api/plates/import") return { status: 201, json: plate };
    if (path === `/api/plates/${id}`) return { json: plate };
  });
  visit("/plates/new");
  await fireEvent.click(
    await screen.findByRole("checkbox", { name: "parts/box.stl" }),
  );
  await fireEvent.click(screen.getByRole("button", { name: /構成を確認/ }));
  await fireEvent.input(await screen.findByLabelText("プレート名"), {
    target: { value: "机の箱" },
  });
  await fireEvent.input(screen.getByLabelText("parts/box.stl の個数"), {
    target: { value: "3" },
  });
  await click("保存");
  await waitFor(() => expect(went).toEqual([`/plates/${id}`]));
  expect(api.to("POST", "/api/plates/import")[0].body).toEqual({
    name: "机の箱",
    conditions: {
      filament_id: null,
      sparse_infill_pattern: null,
      sparse_infill_density: null,
      wall_loops: null,
      brim_enabled: false,
      support_enabled: false,
      support_interface_filament_id: null,
    },
    models: [{ name: "parts/box.stl", source: "parts/box.stl", quantity: 3 }],
  });
  visit(went[0]);
  expect(
    await screen.findByRole("button", { name: "印刷キューへ" }),
  ).toBeTruthy();
  expect(
    await screen.findByRole("button", { name: /parts\/box.stl.*3個/ }),
  ).toBeTruthy();
  expect(screen.getByText("3個", { exact: true })).toBeTruthy();
});

it("late defaults preserve deliberate selections and clearing, without repeated refresh", async () => {
  const machine = "Bambu Lab P1S 0.4 nozzle";
  let reads = 0;
  const late = gate();
  const defaults = {
    default_printer_id: "p1",
    conditions: { filament_id: "white", wall_loops: 2 },
    reason: null,
  };
  const filaments = ["white", "blue"].map((id) => ({
    id,
    name: id,
    vendor: "Test",
    material: "PLA",
  }));
  server(async ({ path, url }) => {
    if (path === "/api/default-settings") {
      reads++;
      await late.held;
      return { json: defaults };
    }
    if (path === "/api/scad/model-info")
      return { json: { roles: ["primary"] } };
    if (path === "/api/scad/models") return { json: ["cube.stl"] };
    if (path === "/api/printers")
      return { json: [{ id: "p1", machine_profile_key: machine }] };
    if (path === "/api/plate-filaments") {
      const selected = url.searchParams.get("selected_id");
      return {
        json: {
          filaments,
          selected: filaments.find((f) => f.id === selected) ?? null,
          selected_state: selected ? "loaded" : "unset",
          loaded_ids: filaments.map((f) => f.id),
          printers: [
            { id: "p1", name: "P1", state: "current", unassigned: false },
          ],
        },
      };
    }
    if (path === "/api/filaments") return { json: filaments };
    if (path.startsWith("/api/filaments/"))
      return { json: { settings: [{ machine_profile_key: machine }] } };
    if (path === "/api/slicer/profiles")
      return {
        json: {
          processes: ["Standard"],
          beds: ["Cool Plate", "High Temp Plate"],
        },
      };
    return { status: 404, json: { error: "not found" } };
  });
  visit("/plates/new");
  await fireEvent.click(await screen.findByRole("checkbox"));
  await fireEvent.click(
    screen.getByRole("button", { name: "構成を確認（1）" }),
  );
  // The printer owns the machine, process and bed; creation asks for none of them.
  for (const name of ["要求する機種・ノズル", "工程（品質）", "ビルドプレート"])
    expect(screen.queryByLabelText(name)).toBeNull();
  await selectMaterial("blue");
  await selectMaterial(null);
  late.release();
  await waitFor(() =>
    expect(screen.queryByText("初期値を読み込んでいます…")).toBeNull(),
  );
  expect(materialButton().textContent).toContain("フィラメントを選択");
  await selectMaterial("blue");
  await fireEvent.click(screen.getByRole("button", { name: "モデル選択へ" }));
  await fireEvent.click(
    screen.getByRole("button", { name: "構成を確認（1）" }),
  );
  await waitFor(() => expect(materialButton().textContent).toContain("blue"));
  expect(reads).toBe(1);
});

const materialButton = () =>
  screen.getByRole("button", { name: /^フィラメント:/ });
const foundMaterial = () =>
  screen.findByRole("button", { name: /^フィラメント:/ });

/** Picks a material in the plate's material dialog, or clears it with null. */
export async function selectMaterial(choice: string | null) {
  await fireEvent.click(await foundMaterial());
  const modal = await screen.findByRole("dialog", {
    name: "フィラメントを選択",
  });
  if (choice === null)
    await fireEvent.click(
      within(modal).getByRole("button", { name: "指定を解除" }),
    );
  else {
    await fireEvent.click(
      within(modal).getByLabelText("所持していないフィラメントを選択する"),
    );
    const button = await waitFor(() => {
      const found = modal.querySelector<HTMLButtonElement>(
        `button[data-filament-id="${choice}"]`,
      );
      expect(found).toBeTruthy();
      return found!;
    });
    await fireEvent.click(button);
  }
  await waitFor(() =>
    expect(
      screen.queryByRole("dialog", { name: "フィラメントを選択" }),
    ).toBeNull(),
  );
}

it("cancel paths restore focus without creating; failed and repeated submits retain the name", async () => {
  let count = 0;
  const hold = gate();
  let duplicate: () => Promise<{
    status: number;
    json: unknown;
  }> = async () => {
    count++;
    await hold.held;
    return { status: 503, json: { error: "Storage unavailable" } };
  };
  server(({ path }) => {
    if (path.endsWith("/duplicate")) return duplicate();
    return {
      json:
        path === "/api/plates"
          ? [
              {
                id,
                version: 1,
                name: "天馬ルームケース: base前",
                models: [
                  {
                    id: "item",
                    name: "bin.stl",
                    source: "bin.stl",
                    quantity: 10,
                  },
                ],
                conditions: {},
              },
            ]
          : [],
    };
  });
  visit("/plates");
  const row = await screen.findByRole("link", { name: /天馬ルームケース/ });
  await press("F10", row, { shiftKey: true });
  const dialog = await screen.findByRole("dialog");
  await fireEvent.click(within(dialog).getByRole("button", { name: "複製" }));
  const name = within(dialog).getByLabelText<HTMLInputElement>("プレート名");
  await waitFor(() => expect(document.activeElement).toBe(name));
  expect(name.value).toBe("天馬ルームケース: base前");
  for (const cancel of ["キャンセル", "Escape", "閉じる"]) {
    if (cancel === "Escape") await press("Escape");
    else
      await fireEvent.click(
        within(dialog)
          .getAllByRole("button", { name: cancel })
          .find((b) => !b.classList.contains("scrim"))!,
      );
    const again = within(dialog).getByRole("button", { name: "複製" });
    await waitFor(() => expect(document.activeElement).toBe(again));
    expect(count).toBe(0);
    await fireEvent.click(again);
    await waitFor(() =>
      expect(document.activeElement).toBe(
        within(dialog).getByLabelText("プレート名"),
      ),
    );
  }
  const input = () =>
    within(dialog).getByLabelText<HTMLInputElement>("プレート名");
  const submit = () =>
    within(dialog).getByRole<HTMLButtonElement>("button", { name: "複製" });
  await fireEvent.input(input(), { target: { value: "" } });
  await fireEvent.click(submit());
  expect(count).toBe(0);
  expect(document.activeElement).toBe(input());
  await fireEvent.input(input(), {
    target: { value: "天馬ルームケース: base後" },
  });
  submit().click();
  submit().click();
  await waitFor(() => expect(count).toBe(1));
  await waitFor(() => expect(input().disabled).toBe(true));
  for (const close of within(dialog).getAllByRole<HTMLButtonElement>("button", {
    name: "閉じる",
  }))
    expect(close.disabled).toBe(true);
  await press("Escape");
  expect(screen.getByRole("dialog")).toBe(dialog);
  await press("Tab");
  expect(document.activeElement).toBe(dialog);
  hold.release();
  expect((await within(dialog).findByRole("alert")).textContent).toContain(
    "保存先",
  );
  expect(input().value).toBe("天馬ルームケース: base後");
  await waitFor(() => expect(document.activeElement).toBe(input()));
  duplicate = async () => ({
    status: 400,
    json: {
      error: "Name must contain 1–256 bytes without control characters",
    },
  });
  await fireEvent.input(input(), { target: { value: "あ".repeat(86) } });
  await fireEvent.click(submit());
  await waitFor(() =>
    expect(within(dialog).getByRole("alert").textContent).toContain(
      "プレート名",
    ),
  );
  expect(input().value).toBe("あ".repeat(86));
});

/** The default printer, its process and the server's shells for the walls asked. */
function strengthServer(defaultsHeld?: Promise<void>) {
  const patterns = Array.from({ length: 26 }, (_, i) =>
    i === 0
      ? "adaptivecubic"
      : i === 1
        ? "gyroid"
        : i === 2
          ? "crosshatch"
          : `pattern-${i}`,
  );
  const shells: Record<string, [number, number]> = {
    "2": [5, 3],
    "3": [8, 5],
    "4": [10, 6],
  };
  const legacy = {
    id,
    version: 1,
    name: "以前のプレート",
    models: [{ id: "item", name: "cube.stl", source: "cube.stl", quantity: 1 }],
    conditions: {
      filament_id: null,
      sparse_infill_pattern: null,
      sparse_infill_density: null,
      wall_loops: null,
    },
  };
  return server(async ({ path, url }) => {
    if (path === "/api/default-settings") {
      await defaultsHeld;
      return {
        json: {
          default_printer_id: "p1",
          conditions: {
            filament_id: null,
            sparse_infill_pattern: "adaptivecubic",
            sparse_infill_density: 15,
            wall_loops: 2,
          },
          infill_patterns: patterns,
          reason: null,
        },
      };
    }
    if (path === "/api/printers")
      return {
        json: [
          {
            id: "p1",
            name: "P1S",
            machine_profile_key: "Bambu Lab P1S 0.4 nozzle",
            default_process_profile_key: "0.20mm Standard @BBL X1C",
          },
        ],
      };
    if (path === "/api/slicer/process") {
      const [top, bottom] = shells[
        url.searchParams.get("wall_loops") ?? "2"
      ] ?? [5, 3];
      return {
        json: {
          top_shell_layers: String(top),
          bottom_shell_layers: String(bottom),
          sparse_infill_pattern: "crosshatch",
          sparse_infill_density: "15%",
          wall_loops: "2",
        },
      };
    }
    if (path === "/api/scad/model-info")
      return { json: { roles: ["primary"] } };
    if (path === "/api/scad/models") return { json: ["cube.stl"] };
    if (path === `/api/plates/${id}`) return { json: legacy };
  });
}
const details = () =>
  [...document.querySelectorAll("details")].find(
    (d) => d.querySelector("summary")?.textContent?.trim() === "詳細設定",
  )!;
const field = (label: string) => screen.getByLabelText<HTMLInputElement>(label);
const pattern = () =>
  screen.getByRole<HTMLSelectElement>("combobox", { name: "インフィル" });
const shellsSay = (text: string) =>
  waitFor(() =>
    expect(within(details()).getByRole("status").textContent).toContain(text),
  );
const typeInto = (input: HTMLInputElement, value: string) =>
  fireEvent.input(input, { target: { value } });

it("details stay optional, persist strength and calculate shells", async () => {
  const api = strengthServer();
  api.route(({ path }) => {
    if (path === "/api/plates/import")
      return { status: 201, json: { id, version: 1, name: "x", models: [] } };
  });
  visit("/plates/new");
  await fireEvent.click(await screen.findByRole("checkbox"));
  await click("構成を確認（1）");
  await waitFor(() => expect(details()).toBeTruthy());
  expect(details().open).toBe(false);
  await typeInto(field("プレート名"), "充填・壁設定");
  await fireEvent.click(details().querySelector("summary")!);
  expect(details().open).toBe(true);
  const brim = field("ブリムを付ける");
  expect(brim.checked).toBe(false);
  await fireEvent.click(brim);
  await waitFor(() => expect(pattern().value).toBe("adaptivecubic"));
  expect(field("充填率（%）").value).toBe("15");
  expect(field("壁の枚数（周）").value).toBe("2");
  expect(pattern().options.length).toBe(27);
  await shellsSay("上面5層・底面3層");
  await typeInto(field("壁の枚数（周）"), "4");
  await shellsSay("上面10層・底面6層");
  await typeInto(field("壁の枚数（周）"), "3");
  await shellsSay("上面8層・底面5層");
  // The default printer's process gives the inherited layers; the plate has no process.
  expect(screen.queryByLabelText("工程（品質）")).toBeNull();
  await fireEvent.change(pattern(), { target: { value: "gyroid" } });
  await typeInto(field("充填率（%）"), "22.5");
  await shellsSay("上面8層・底面5層");
  await click("保存");
  await waitFor(() =>
    expect(api.to("POST", "/api/plates/import")).toHaveLength(1),
  );
  expect(api.to("POST", "/api/plates/import")[0].body.conditions).toMatchObject(
    {
      sparse_infill_pattern: "gyroid",
      sparse_infill_density: 22.5,
      wall_loops: 3,
      brim_enabled: true,
    },
  );
});

it("late defaults and failed saves preserve manual input; legacy values come from the process", async () => {
  const late = gate();
  const api = strengthServer(late.held);
  let fail = true;
  api.route(({ path }) => {
    if (path !== "/api/plates/import") return;
    if (fail) return { status: 503, json: {} };
    return { status: 201, json: { id, version: 1, name: "x", models: [] } };
  });
  visit("/plates/new");
  await fireEvent.click(await screen.findByRole("checkbox"));
  await click("構成を確認（1）");
  await waitFor(() => expect(details()).toBeTruthy());
  await fireEvent.click(details().querySelector("summary")!);
  await typeInto(field("壁の枚数（周）"), "4");
  await typeInto(field("充填率（%）"), "0");
  await fireEvent.click(field("ブリムを付ける"));
  late.release();
  await waitFor(() => expect(pattern().value).toBe("adaptivecubic"));
  expect(field("壁の枚数（周）").value).toBe("4");
  expect(field("充填率（%）").value).toBe("0");
  await typeInto(field("プレート名"), "入力を保持");
  await click("保存");
  await waitFor(() =>
    expect(screen.getAllByRole("alert").some((a) => a.closest("form"))).toBe(
      true,
    ),
  );
  expect(field("プレート名").value).toBe("入力を保持");
  expect(field("ブリムを付ける").checked).toBe(true);
  expect(field("壁の枚数（周）").value).toBe("4");
  expect(field("充填率（%）").value).toBe("0");
  fail = false;
  await click("保存");
  await waitFor(() => expect(went).toEqual([`/plates/${id}`]));
  // A plate saved before the defaults shows the process's values and saves them.
  visit(`/plates/${id}`);
  await click("構成を編集");
  await waitFor(() => expect(details()).toBeTruthy());
  await fireEvent.click(details().querySelector("summary")!);
  await waitFor(() => expect(pattern().value).toBe("crosshatch"));
  expect(field("壁の枚数（周）").value).toBe("2");
  await click("保存");
  await waitFor(() =>
    expect(api.to("PUT", `/api/plates/${id}`)).toHaveLength(1),
  );
  expect(api.to("PUT", `/api/plates/${id}`)[0].body.conditions).toMatchObject({
    sparse_infill_pattern: "crosshatch",
    sparse_infill_density: 15,
    wall_loops: 2,
  });
});
