import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, reset, visit } from "../testing/page";
import { server } from "../testing/server";

afterEach(reset);

const id = "11111111-1111-4111-8111-111111111111";
const front = "gridfinity/base-front.stl",
  back = "gridfinity/base-back.stl",
  bin = "gridfinity/bin.stl";
const conditions = {
  filament_id: "white",
  wall_loops: 4,
  sparse_infill_density: 30,
  sparse_infill_pattern: "gyroid",
  brim_enabled: true,
  support_enabled: true,
  support_interface_filament_id: "blue",
};
const materials = ["white", "blue", "second"].map((m) => ({
  id: m,
  name: `PLA ${m}`,
  vendor: "Fixture",
  material: "PLA",
}));

/** A duplicated plate of ten base-fronts, and the catalog it can take models from. */
function editor(conflict = false) {
  let plate: any = {
    id,
    version: 1,
    name: "天馬ルームケース: base後",
    models: [{ id: "front", name: front, source: front, quantity: 10 }],
    conditions,
  };
  const api = server(({ method, path, url, body }) => {
    if (path === `/api/plates/${id}`) {
      if (method === "PUT") {
        if (conflict) return { status: 409, json: { error: "version" } };
        plate = { ...plate, ...body, version: plate.version + 1 };
      }
      return { json: plate };
    }
    if (path === "/api/scad/models") {
      const q = url.searchParams.get("q") ?? "";
      return { json: [front, back, bin].filter((m) => m.includes(q)) };
    }
    if (path === "/api/scad/model-info")
      return { json: { roles: ["primary"] } };
    if (path === "/api/default-settings")
      return {
        json: {
          default_printer_id: "p1",
          conditions: { filament_id: null },
          infill_patterns: ["gyroid", "adaptivecubic", "crosshatch"],
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
    if (path === "/api/plate-filaments") {
      const selected = materials.find(
        (m) => m.id === url.searchParams.get("selected_id"),
      );
      return {
        json: {
          filaments: materials,
          selected: selected ?? null,
          selected_state: selected ? "loaded" : "unset",
          loaded_ids: materials.map((m) => m.id),
          printers: [
            { id: "p1", name: "P1S", state: "current", unassigned: false },
          ],
        },
      };
    }
  });
  return { api, plate: () => plate };
}
const field = (label: string) =>
  screen.getByLabelText<HTMLInputElement & HTMLSelectElement>(label);
const type = (label: string, value: string) =>
  fireEvent.input(field(label), { target: { value } });
const search = () =>
  screen.getByRole<HTMLInputElement>("searchbox", { name: "モデル名で検索" });
const find = async (q: string) => {
  await fireEvent.input(search(), { target: { value: q } });
};
const focused = (element: () => Element) =>
  waitFor(() => expect(document.activeElement).toBe(element()));

it("add and replace keep quantity, drafts and conditions", async () => {
  const { api, plate } = editor();
  visit(`/plates/${id}?edit=1`);
  await waitFor(() =>
    expect(field("プレート名").value).toBe("天馬ルームケース: base後"),
  );
  await type("プレート名", "別規格");
  const details = [...document.querySelectorAll("details")].find(
    (d) => d.querySelector("summary")?.textContent?.trim() === "詳細設定",
  )!;
  await fireEvent.click(details.querySelector("summary")!);
  await type("壁の枚数（周）", "5");
  await type("充填率（%）", "25");
  await fireEvent.change(screen.getByRole("combobox", { name: "インフィル" }), {
    target: { value: "adaptivecubic" },
  });
  await fireEvent.click(field("ブリムを付ける"));
  await fireEvent.click(
    screen.getByRole("button", { name: /^接触面のフィラメント:/ }),
  );
  const option = await waitFor(() => {
    const found = screen
      .getByRole("dialog")
      .querySelector<HTMLButtonElement>('button[data-filament-id="second"]');
    expect(found).toBeTruthy();
    return found!;
  });
  await fireEvent.click(option);
  // Replacing a model keeps its quantity and the draft.
  await click(`${front}を差し替え`);
  await focused(search);
  expect(
    (
      await screen.findByRole<HTMLButtonElement>("button", {
        name: `${front} 選択済み`,
      })
    ).disabled,
  ).toBe(true);
  await find("base-back");
  await click(back);
  await focused(() =>
    screen.getByRole("button", { name: `${back}を差し替え` }),
  );
  expect(field(`${back} の個数`).value).toBe("10");
  expect(field("プレート名").value).toBe("別規格");
  expect(field("壁の枚数（周）").value).toBe("5");
  expect(field("充填率（%）").value).toBe("25");
  expect(
    screen.getByRole<HTMLSelectElement>("combobox", { name: "インフィル" })
      .value,
  ).toBe("adaptivecubic");
  expect(field("ブリムを付ける").checked).toBe(false);
  expect(api.to("PUT", `/api/plates/${id}`)).toHaveLength(0);
  // Adding offers what is not there yet; cancelling adds nothing.
  await click("モデルを追加");
  await find("bin");
  await fireEvent.click(await screen.findByRole("checkbox", { name: bin }));
  await find("base");
  await waitFor(() =>
    expect(
      screen.getByRole<HTMLInputElement>("checkbox", { name: back }).disabled,
    ).toBe(true),
  );
  await click("キャンセル");
  await focused(() => screen.getByRole("button", { name: "モデルを追加" }));
  expect(screen.queryByLabelText(`${bin} の個数`)).toBeNull();
  await click("モデルを追加");
  await find("bin");
  await fireEvent.click(await screen.findByRole("checkbox", { name: bin }));
  await click("追加（1）");
  await waitFor(() => expect(field(`${bin} の個数`).value).toBe("1"));
  expect(field(`${back} の個数`).value).toBe("10");
  await click(`${back}を差し替え`);
  await find("");
  await waitFor(() =>
    expect(
      screen.getByRole<HTMLButtonElement>("button", { name: `${bin} 選択済み` })
        .disabled,
    ).toBe(true),
  );
  await click("キャンセル");
  await click("保存");
  await waitFor(() =>
    expect(api.to("PUT", `/api/plates/${id}`)).toHaveLength(1),
  );
  const saved = api.to("PUT", `/api/plates/${id}`)[0].body;
  expect(saved.name).toBe("別規格");
  expect(saved.models.map((m: any) => [m.source, m.quantity])).toEqual([
    [back, 10],
    [bin, 1],
  ]);
  expect(saved.conditions).toMatchObject({
    ...conditions,
    wall_loops: 5,
    sparse_infill_density: 25,
    sparse_infill_pattern: "adaptivecubic",
    brim_enabled: false,
    support_interface_filament_id: "second",
  });
  // Cancelling the whole editor saves nothing, even after an empty composition.
  await click("構成を編集");
  for (const model of [back, bin]) await click(`${model}を構成から外す`);
  expect(
    screen.getByRole<HTMLButtonElement>("button", { name: "保存" }).disabled,
  ).toBe(true);
  await click("モデルを追加");
  await find("front");
  await fireEvent.click(await screen.findByRole("checkbox", { name: front }));
  await click("追加（1）");
  await click("編集をやめる");
  expect(api.to("PUT", `/api/plates/${id}`)).toHaveLength(1);
  expect(plate().models.map((m: any) => m.source)).toEqual([back, bin]);
});

it("a version conflict preserves the edited composition", async () => {
  editor(true);
  visit(`/plates/${id}?edit=1`);
  await waitFor(() =>
    expect(field("プレート名").value).toBe("天馬ルームケース: base後"),
  );
  await type("プレート名", "未保存の変更");
  await click(`${front}を差し替え`);
  await find("base-back");
  await click(back);
  await click("保存");
  await waitFor(() =>
    expect(
      screen.getAllByRole("alert").some((a) => a.textContent?.includes("更新")),
    ).toBe(true),
  );
  expect(field("プレート名").value).toBe("未保存の変更");
  expect(field(`${back} の個数`).value).toBe("10");
  expect(within(document.body).queryByRole("dialog")).toBeNull();
});
