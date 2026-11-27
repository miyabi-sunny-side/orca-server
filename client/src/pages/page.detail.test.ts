import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, reset, visit, went } from "../testing/page";
import { server } from "../testing/server";

afterEach(reset);

const id = "11111111-1111-4111-8111-111111111111";
const emptyDefaults = {
  default_printer_id: null,
  conditions: { filament_id: null },
  reason: "printer",
};
/** Long enough for the estimate's two-second refresh. */
const polled = { timeout: 5000 };

it("plate calculation states and recovery", async () => {
  const plate = {
    id,
    version: 1,
    name: "キューブの保存済みプレート",
    models: [{ id: "item", name: "cube.stl", source: "cube.stl", quantity: 2 }],
    conditions: { filament_id: "pla" },
  };
  let state: Record<string, unknown> = {
      state: "calculating",
      seconds: null,
      error: null,
    },
    mini: Record<string, unknown> | null = null,
    retries = 0;
  const p1 = {
    printer_id: "p1",
    printer_name: "P1S",
    machine_profile_key: "P1S 0.4",
  };
  server(({ method, path }) => {
    if (path.endsWith("/slice")) {
      if (method === "POST") {
        retries++;
        state = { state: "calculating", seconds: null, error: null };
        return { status: 202 };
      }
      return {
        json: {
          printers: [
            ...(mini
              ? [
                  {
                    printer_id: "a1",
                    printer_name: "A1 mini",
                    machine_profile_key: "A1 mini 0.4",
                    ...mini,
                  },
                ]
              : []),
            { ...p1, ...state },
          ],
        },
      };
    }
    if (path === `/api/plates/${id}`) return { json: plate };
    if (path === "/api/filaments")
      return { json: [{ id: "pla", name: "PLA 青" }] };
  });
  visit(`/plates/${id}`);
  expect(await screen.findByRole("heading", { name: plate.name })).toBeTruthy();
  const status = () => screen.getByLabelText("プレートの試算");
  const says = (text: string) =>
    waitFor(() => expect(status().textContent).toContain(text), polled);
  await says("試算中…");
  state = { state: "ready", seconds: 1140, error: null };
  await says("約19分");
  state = {
    state: "failed",
    seconds: null,
    error:
      "Selected build plate temperature is missing or zero for this material",
  };
  await says("試算できませんでした");
  await fireEvent.click(status().querySelector("summary")!);
  expect(
    within(status())
      .getByRole("link", { name: /材料の設定/ })
      .getAttribute("href"),
  ).toMatch(/\/filaments\/pla/);
  await click("再試算", status());
  await waitFor(() => expect(retries).toBe(1));
  await says("試算中…");
  state = { state: "ready", seconds: 1140, error: null };
  await says("約19分");
  // One printer keeps the single line; it does not name the printer.
  expect(status().textContent).not.toContain("P1S");
  // With a second printer each line names its printer and why it cannot print.
  mini = {
    state: "failed",
    seconds: null,
    error: "Models must fit together on one plate",
    reason: "unfit",
  };
  const rows = () => within(status()).getAllByRole("listitem");
  await waitFor(() => expect(rows()).toHaveLength(2), polled);
  expect(rows()[0].textContent).toContain("A1 mini · 台に乗りません");
  expect(rows()[1].textContent).toContain("P1S · 約19分");
  await fireEvent.click(rows()[0].querySelector("summary")!);
  expect(within(rows()[0]).getByRole("alert").textContent).toContain(
    "台に乗りません",
  );
  expect(
    within(rows()[0]).queryByRole("link", { name: /材料の設定/ }),
  ).toBeNull();
  mini = {
    state: "failed",
    seconds: null,
    error: "Configure this material for the required machine and nozzle first",
    reason: "material_setting",
  };
  await waitFor(
    () =>
      expect(rows()[0].textContent).toContain("A1 mini · 材料設定がありません"),
    polled,
  );
  expect(
    within(rows()[0])
      .getByRole("link", { name: /材料の設定/ })
      .getAttribute("href"),
  ).toMatch(/\/filaments\/pla\?machine=A1%20mini%200\.4/);
}, 20_000);

it("WebGL unavailable keeps composition editing usable", async () => {
  // jsdom has no WebGL, like a browser where it is turned off.
  const plate = {
    id,
    version: 1,
    name: "形状を見て選ぶプレート",
    models: [
      {
        id: "cube",
        name: "立方体.stl",
        source: "parts/cube.stl",
        quantity: 10,
      },
      { id: "triangle", name: "三角形.stl", source: null, quantity: 1 },
    ],
  };
  server(({ path }) => {
    if (path === "/api/default-settings") return { json: emptyDefaults };
    if (path === `/api/plates/${id}`) return { json: plate };
  });
  visit(`/plates/${id}`);
  await waitFor(() =>
    expect(
      screen
        .getAllByRole("alert")
        .some((a) => a.textContent?.includes("WebGL")),
    ).toBe(true),
  );
  await click("構成を編集");
  await waitFor(() =>
    expect(screen.getByLabelText<HTMLInputElement>("プレート名").value).toBe(
      plate.name,
    ),
  );
});

it("search retains selected models across filtering and saving", async () => {
  const names = Array.from(
    { length: 100 },
    (_, i) => `parts/item-${String(i).padStart(3, "0")}.stl`,
  );
  let saved: any;
  server(({ path, url, body }) => {
    if (path === "/api/default-settings") return { json: emptyDefaults };
    if (path === "/api/scad/model-info")
      return { json: { roles: ["primary"] } };
    if (path === "/api/scad/models")
      return {
        json: names.filter((n) => n.includes(url.searchParams.get("q") ?? "")),
      };
    if (path === "/api/plates/import") {
      saved = body;
      return { status: 201, json: { id, version: 1, ...saved } };
    }
  });
  visit("/plates/new");
  await fireEvent.click(
    await screen.findByRole("checkbox", { name: names[0] }),
  );
  const search = screen.getByRole<HTMLInputElement>("searchbox", {
    name: "モデル名で検索",
  });
  await fireEvent.input(search, { target: { value: "099" } });
  await fireEvent.click(
    await screen.findByRole("checkbox", { name: names[99] }),
  );
  await click("構成を確認（2）");
  await fireEvent.input(screen.getByLabelText("プレート名"), {
    target: { value: "選択を保持" },
  });
  await fireEvent.input(screen.getByLabelText(names[0] + " の個数"), {
    target: { value: "10" },
  });
  await click("モデル選択へ");
  expect(search.value).toBe("099");
  expect(
    (await screen.findByRole<HTMLInputElement>("checkbox", { name: names[99] }))
      .checked,
  ).toBe(true);
  await click("構成を確認（2）");
  expect(
    screen.getByLabelText<HTMLInputElement>(names[0] + " の個数").value,
  ).toBe("10");
  await click("保存");
  await waitFor(() => expect(went).toEqual([`/plates/${id}`]));
  expect(saved.models.map((m: any) => [m.source, m.quantity])).toEqual([
    [names[0], 10],
    [names[99], 1],
  ]);
});
