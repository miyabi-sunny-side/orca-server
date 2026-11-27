import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, reset, visit, went } from "../testing/page";
import { server } from "../testing/server";

afterEach(reset);

const machine = "Bambu Lab P1S 0.4 nozzle";

/** The material catalog: products with their colors and per-machine settings, kept in place. */
function catalog() {
  const products: any[] = [
    {
      id: "matte",
      name: "PLA Matte 黒",
      vendor: "Fixture",
      material: "PLA",
      bambu_filament_id: null,
      colors: [{ id: "black", name: "PLA Matte 黒", color: "000000FF" }],
      settings: [],
    },
    {
      id: "white-product",
      name: "PLA Matte 白",
      vendor: "Fixture",
      material: "PLA",
      bambu_filament_id: null,
      colors: [{ id: "white", name: "PLA Matte 白", color: "FFFFFFFF" }],
      settings: [],
    },
  ];
  let failList = false,
    failSetting = false,
    referenced = false,
    serial = 0;
  const find = (id: string) => products.find((p) => p.id === id);
  const resolved = (s: any) => ({
    ...s,
    resolved: {
      nozzle_temperature_initial_layer: String(
        s.overrides_json.nozzle_temperature_initial_layer,
      ),
      nozzle_temperature: String(s.overrides_json.nozzle_temperature),
      required_nozzle_hrc: null,
    },
  });
  const api = server(({ method, path, body }) => {
    if (path === "/api/filament-products") {
      if (method === "POST") {
        const product = {
          ...body,
          id: `product-${++serial}`,
          colors: [],
          settings: [],
        };
        products.push(product);
        return { status: 201, json: product };
      }
      if (failList) return { status: 503, json: { error: "unavailable" } };
      return { json: products };
    }
    if (path === "/api/printers/profiles")
      return {
        json: [
          { key: machine, model: "Bambu Lab P1S", nozzle_diameter: "0.4" },
        ],
      };
    const parts = path.split("/");
    const product = find(parts[3]);
    if (path.endsWith("/profiles"))
      return {
        json: ["Generic PETG", "Generic PLA"].map((key) => ({
          key,
          resolved: {
            nozzle_temperature_initial_layer: "255",
            nozzle_temperature: "250",
            required_nozzle_hrc: null,
          },
        })),
      };
    if (path.startsWith("/api/filaments/")) {
      const owner = products.find((p) =>
        p.colors.some((c: any) => c.id === parts[3]),
      );
      return { json: { product_id: owner.id, settings: owner.settings } };
    }
    if (!product) return { status: 404, json: { error: "not found" } };
    if (parts[4] === "colors") {
      if (method === "POST")
        product.colors.push({ ...body, id: `color-${++serial}` });
      if (method === "PUT")
        Object.assign(
          product.colors.find((c: any) => c.id === parts[5]),
          body,
        );
      return { status: 204 };
    }
    if (parts[4] === "settings") {
      if (method === "POST") {
        if (failSetting) return { status: 409, json: { error: "duplicate" } };
        product.settings.push(resolved({ ...body, id: `setting-${++serial}` }));
      }
      if (method === "PUT") {
        const at = product.settings.findIndex((s: any) => s.id === parts[5]);
        product.settings[at] = resolved({ ...body, id: parts[5] });
      }
      if (method === "DELETE")
        product.settings = product.settings.filter(
          (s: any) => s.id !== parts[5],
        );
      return { status: 204 };
    }
    if (parts[4] === "adopt") {
      const from = products.find((p) =>
        p.colors.some((c: any) => c.id === body.filament_id),
      );
      product.colors.push(...from.colors);
      products.splice(products.indexOf(from), 1);
      return { status: 204 };
    }
    if (method === "DELETE") {
      if (referenced) return { status: 409, json: { error: "referenced" } };
      products.splice(products.indexOf(product), 1);
      return { status: 204 };
    }
    return { json: product };
  });
  return {
    api,
    products,
    failList: (v: boolean) => (failList = v),
    failSetting: (v: boolean) => (failSetting = v),
    reference: () => (referenced = true),
  };
}
const field = (label: string) =>
  screen.getByLabelText<HTMLInputElement & HTMLSelectElement>(label);
const type = (label: string, value: string) =>
  fireEvent.input(field(label), { target: { value } });
/** Opens where the page asked to go last. */
const follow = () => visit(went.at(-1)!);
const section = (name: string) =>
  [...document.querySelectorAll("details")].find(
    (d) => d.querySelector("summary")?.textContent?.trim() === name,
  )!;

it("material CRUD, temperatures and merged colors", async () => {
  const c = catalog();
  window.confirm = () => true;
  c.failList(true);
  visit("/filaments");
  expect((await screen.findByRole("alert")).textContent).toContain("保存先");
  c.failList(false);
  await click("読み直す");
  expect(
    await screen.findByRole("link", { name: /PLA Matte 黒/ }),
  ).toBeTruthy();
  visit(screen.getByRole("link", { name: "製品を追加" }).getAttribute("href")!);
  const name = "温度調整用の長い製品名・試験フィラメント";
  await waitFor(() => expect(field("製品名")).toBeTruthy());
  await type("製品名", name);
  await type("メーカー", "Example vendor");
  await type("材料種別", "PETG");
  await click("保存");
  await waitFor(() => expect(went).toHaveLength(1));
  const id = went[0].split("/").pop()!;
  for (const color of ["黒", "白", "黄"]) {
    visit(`/filaments/${id}/colors/new`);
    await waitFor(() => expect(field("基本色")).toBeTruthy());
    const preset = [...field("基本色").options].find(
      (o) => o.textContent?.trim() === color,
    )!;
    await fireEvent.change(field("基本色"), {
      target: { value: preset.value },
    });
    expect(field("色名").value).toBe(color);
    expect(screen.queryByLabelText("メーカー")).toBeNull();
    await click("保存");
    await waitFor(() => expect(went.at(-1)).toBe(`/filaments/${id}`));
  }
  follow();
  expect(await screen.findByRole("link", { name: "黒" })).toBeTruthy();
  visit(screen.getByRole("link", { name: "設定を追加" }).getAttribute("href")!);
  await waitFor(() =>
    expect(field("機種・ノズル径").options.length).toBeGreaterThan(1),
  );
  await fireEvent.change(field("機種・ノズル径"), {
    target: { value: machine },
  });
  await waitFor(() =>
    expect(
      field("基本のフィラメントプロファイル").options.length,
    ).toBeGreaterThan(1),
  );
  await fireEvent.change(field("基本のフィラメントプロファイル"), {
    target: { value: "Generic PETG" },
  });
  await type("初層（℃）", "250");
  await type("通常層（℃）", "240");
  await type("ベッド初層（℃）", "65");
  await type("ベッド通常層（℃）", "0");
  c.failSetting(true);
  await click("保存");
  expect((await screen.findByRole("alert")).textContent).toContain("同じ機種");
  expect(field("初層（℃）").value).toBe("250");
  c.failSetting(false);
  const before = went.length;
  await click("保存");
  await waitFor(() => expect(went).toHaveLength(before + 1));
  expect(
    c.api.to("POST", `/api/filament-products/${id}/settings`).at(-1)!.body,
  ).toEqual({
    machine_profile_key: machine,
    base_profile_key: "Generic PETG",
    overrides_json: {
      nozzle_temperature_initial_layer: 250,
      nozzle_temperature: 240,
      bed_temperature_initial_layer: 65,
      bed_temperature: 0,
    },
  });
  follow();
  const setting = await screen.findByRole("link", {
    name: /初層 250℃ \/ 通常 240℃/,
  });
  visit(setting.getAttribute("href")!);
  await waitFor(() => expect(field("初層（℃）").value).toBe("250"));
  await type("初層（℃）", "251");
  await type("ベッド初層（℃）", "");
  await type("ベッド通常層（℃）", "");
  await click("保存");
  await waitFor(() =>
    expect(c.api.to("PUT", /\/settings\/setting-/)).toHaveLength(1),
  );
  // Emptied bed temperatures go as null, which the server drops.
  expect(
    c.api.to("PUT", /\/settings\/setting-/)[0].body.overrides_json,
  ).toEqual({
    nozzle_temperature_initial_layer: 251,
    nozzle_temperature: 240,
    bed_temperature_initial_layer: null,
    bed_temperature: null,
  });
  follow();
  expect(await screen.findByRole("link", { name: /初層 251℃/ })).toBeTruthy();
  // The exact color of one color.
  visit(screen.getByRole("link", { name: "黒" }).getAttribute("href")!);
  await waitFor(() => expect(section("正確な色を編集")).toBeTruthy());
  await fireEvent.click(section("正確な色を編集").querySelector("summary")!);
  await type("色（RGBA・8桁）", "161616FF");
  await type("色名", "チャコール");
  await click("保存");
  follow();
  expect(await screen.findByRole("link", { name: "チャコール" })).toBeTruthy();
  expect(
    c.products
      .find((p) => p.id === id)
      .colors.find((x: any) => x.name === "チャコール").color,
  ).toBe("161616FF");
  // Removing the setting, then the product.
  visit(screen.getByRole("link", { name: /初層 251℃/ }).getAttribute("href")!);
  await click("この設定を削除");
  follow();
  expect(
    await screen.findByText(
      "使う機種・ノズル径ごとに基本プロファイルを選択してください。",
    ),
  ).toBeTruthy();
  await fireEvent.click(section("製品の管理").querySelector("summary")!);
  await click("製品を削除");
  await waitFor(() => expect(went.at(-1)).toBe("/filaments"));
  expect(c.products.some((p) => p.id === id)).toBe(false);
  // A product the AMS or a job refers to stays.
  c.reference();
  visit("/filaments/white-product");
  await waitFor(() => expect(section("製品の管理")).toBeTruthy());
  await fireEvent.click(section("製品の管理").querySelector("summary")!);
  await click("製品を削除");
  expect((await screen.findByRole("alert")).textContent).toContain("参照");
  // Merging a color of another product into this one; the old address leads to it.
  visit("/filaments/matte");
  await waitFor(() => expect(section("既存の色をまとめる")).toBeTruthy());
  await fireEvent.click(
    section("既存の色をまとめる").querySelector("summary")!,
  );
  await fireEvent.change(screen.getByRole("combobox", { name: "既存の色" }), {
    target: { value: "white" },
  });
  await click("この製品にまとめる");
  expect(
    await screen.findByRole("link", { name: "PLA Matte 白" }),
  ).toBeTruthy();
  visit("/filaments/white");
  await waitFor(() =>
    expect(went.at(-1)).toBe("/filaments/matte/colors/white"),
  );
});
