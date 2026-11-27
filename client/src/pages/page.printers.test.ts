import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, reload, reset, visit, went } from "../testing/page";
import { server } from "../testing/server";

afterEach(reset);

it("default printer selection persists across reload among same-model printers", async () => {
  const machine = "Bambu Lab P1S 0.4 nozzle";
  let chosen: string | null = "p1";
  const api = server(({ method, path, body }) => {
    if (path === "/api/printers")
      return {
        json: ["p1", "p2"].map((id) => ({
          id,
          name: `P1S ${id}`,
          machine_profile_key: machine,
          status: { connection: "connected" },
        })),
      };
    if (path === "/api/default-settings") {
      if (method === "PUT") {
        chosen = body.default_printer_id;
        return { status: 204 };
      }
      return {
        json: {
          default_printer_id: chosen,
          conditions: { filament_id: null },
          infill_patterns: ["crosshatch"],
          reason: null,
        },
      };
    }
  });
  const select = () =>
    screen.findByLabelText<HTMLSelectElement>(
      "新規プレートの初期値に使うプリンター",
    );
  visit("/printers");
  await waitFor(async () => expect((await select()).value).toBe("p1"));
  await fireEvent.change(await select(), { target: { value: "p2" } });
  expect(
    (await screen.findByText("初期値に使うプリンターを保存しました。"))
      .textContent,
  ).toBe("初期値に使うプリンターを保存しました。");
  expect(api.to("PUT", "/api/default-settings")[0].body).toEqual({
    default_printer_id: "p2",
  });
  reload();
  await waitFor(async () => expect((await select()).value).toBe("p2"));
});

it("saved defaults only affect new plates and survive reload", async () => {
  const machine = "Bambu Lab P1S 0.4 nozzle";
  let conditions: Record<string, unknown> = {
    filament_id: null,
    sparse_infill_pattern: "adaptivecubic",
    sparse_infill_density: 15,
    wall_loops: 2,
  };
  const api = server(({ method, path, body }) => {
    if (path === "/api/printers")
      return {
        json: [
          {
            id: "p1",
            name: "P1S",
            machine_profile_key: machine,
            status: { connection: "connected" },
          },
        ],
      };
    if (path === "/api/default-settings") {
      if (method === "PUT") {
        const { default_printer_id: _, ...strength } = body;
        conditions = { ...conditions, ...strength };
        return { status: 204 };
      }
      return {
        json: {
          default_printer_id: "p1",
          conditions,
          infill_patterns: ["adaptivecubic", "gyroid", "crosshatch"],
          reason: null,
        },
      };
    }
  });
  const open = async () => {
    const summary = [...document.querySelectorAll("summary")].find(
      (s) => s.textContent?.trim() === "新規プレートの詳細初期値",
    );
    await fireEvent.click(summary!);
  };
  visit("/printers");
  await waitFor(() =>
    expect(screen.getByRole("combobox", { name: "インフィル" })).toBeTruthy(),
  );
  await open();
  await fireEvent.change(screen.getByRole("combobox", { name: "インフィル" }), {
    target: { value: "gyroid" },
  });
  await fireEvent.input(screen.getByLabelText("壁の枚数（周）"), {
    target: { value: "4" },
  });
  await fireEvent.input(screen.getByLabelText("充填率（%）"), {
    target: { value: "25" },
  });
  await fireEvent.click(screen.getByRole("button", { name: "初期値を保存" }));
  expect(
    await screen.findByText("新規プレートの初期値を保存しました。"),
  ).toBeTruthy();
  expect(api.to("PUT", "/api/default-settings")[0].body).toMatchObject({
    default_printer_id: "p1",
    sparse_infill_pattern: "gyroid",
    wall_loops: 4,
    sparse_infill_density: 25,
  });
  // Only the defaults are written; no plate is.
  expect(api.sent.filter((r) => r.method !== "GET")).toHaveLength(1);
  reload();
  await waitFor(() =>
    expect(
      screen.getByLabelText<HTMLInputElement>("壁の枚数（周）").value,
    ).toBe("4"),
  );
});

it("persistent printer CRUD, nozzle-specific profiles and the selected queue", async () => {
  const mini = "Bambu Lab A1 mini 0.2 nozzle",
    p1s = "Bambu Lab P1S 0.4 nozzle";
  const profiles: Record<string, object> = {
    [mini]: {
      version: "1",
      printer: mini,
      processes: [
        "0.10mm Standard @BBL A1M 0.2 nozzle",
        "0.08mm Extra Fine @BBL A1M 0.2 nozzle",
      ],
      filaments: [],
      beds: ["Textured PEI Plate"],
      defaults: {
        process: "0.10mm Standard @BBL A1M 0.2 nozzle",
        bed: "Textured PEI Plate",
        filament: "",
      },
    },
    [p1s]: {
      version: "1",
      printer: p1s,
      processes: ["0.20mm Standard @BBL X1C", "0.16mm Optimal @BBL X1C"],
      filaments: [],
      beds: ["Cool Plate", "High Temp Plate"],
      defaults: {
        process: "0.20mm Standard @BBL X1C",
        bed: "Cool Plate",
        filament: "",
      },
    },
  };
  let printers: any[] = [];
  let busy = false;
  window.confirm = () => true;
  const api = server(({ method, path, url, body }) => {
    if (path === "/api/printers/profiles")
      return {
        json: [
          { key: mini, model: "Bambu Lab A1 mini", nozzle_diameter: "0.2" },
          { key: p1s, model: "Bambu Lab P1S", nozzle_diameter: "0.4" },
        ],
      };
    if (path === "/api/slicer/profiles")
      return { json: profiles[url.searchParams.get("machine")!] };
    if (path === "/api/default-settings")
      return {
        json: {
          default_printer_id: printers[0]?.id ?? null,
          conditions: { filament_id: null },
          infill_patterns: [],
          reason: null,
        },
      };
    if (path === "/api/printers") {
      if (method === "POST") {
        const { access_code: _, tls_certificate: __, ...stored } = body;
        printers.push({
          ...stored,
          id: "p-1",
          machine: { model: "Bambu Lab A1 mini", nozzle_diameter: "0.2" },
          status: { connection: "connecting" },
        });
        return { status: 201, json: printers[0] };
      }
      return { json: printers };
    }
    const one = printers.find((p) => path === `/api/printers/${p.id}`);
    if (one && method === "PUT") {
      if (busy) return { status: 409, json: { error: "in use" } };
      const { access_code: _, tls_certificate: __, ...stored } = body;
      Object.assign(one, stored);
      return { json: one };
    }
    if (one && method === "DELETE") {
      printers = printers.filter((p) => p !== one);
      return { status: 204 };
    }
    if (one) return { json: one };
  });
  const field = (label: string) =>
    screen.getByLabelText<HTMLInputElement & HTMLSelectElement>(label);
  const type = (label: string, value: string) =>
    fireEvent.input(field(label), { target: { value } });
  visit("/printers");
  expect(
    await screen.findByText(
      "プリンターを追加すると、接続状態の確認と印刷ができます。",
    ),
  ).toBeTruthy();
  visit(screen.getByRole("link", { name: "追加" }).getAttribute("href")!);
  await waitFor(() =>
    expect(field("機種・装着ノズル径").options.length).toBeGreaterThan(1),
  );
  await type("名前", "作業部屋のプリンター・細かい部品のための長い名前");
  await fireEvent.change(field("機種・装着ノズル径"), {
    target: { value: mini },
  });
  await waitFor(() =>
    expect(field("既定の工程").value).toBe(
      "0.10mm Standard @BBL A1M 0.2 nozzle",
    ),
  );
  expect([...field("既定の工程").options].map((o) => o.value)).not.toContain(
    "0.20mm Standard @BBL X1C",
  );
  await fireEvent.change(field("ノズル材質"), {
    target: { value: "stainless_steel" },
  });
  await type("IPアドレス", "127.0.0.1");
  await type("シリアル番号", "UIREGISTRY");
  await type("LANアクセスコード", "12345678");
  await fireEvent.input(field("TLS証明書（PEM）"), {
    target: {
      value: "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----",
    },
  });
  await type("MQTTポート", "1");
  await type("FTPSポート", "1");
  const form = document.querySelector("form")!;
  expect(
    [...form.elements]
      .filter((e: any) => e.willValidate && !e.validity.valid)
      .map((e) => e.outerHTML),
  ).toEqual([]);
  await click("保存");
  await waitFor(() => expect(went).toEqual(["/printers"]));
  expect(api.to("POST", "/api/printers")[0].body).toMatchObject({
    name: "作業部屋のプリンター・細かい部品のための長い名前",
    machine_profile_key: mini,
    default_process_profile_key: "0.10mm Standard @BBL A1M 0.2 nozzle",
    nozzle_material: "stainless_steel",
    host: "127.0.0.1",
    serial: "UIREGISTRY",
    access_code: "12345678",
    mqtt_port: 1,
    ftps_port: 1,
  });
  visit("/printers");
  expect(await screen.findByText(/Bambu Lab A1 mini · 0.2 mm/)).toBeTruthy();
  expect(
    screen.getByRole("link", { name: "キュー" }).getAttribute("href"),
  ).toBe("/queue?printer_id=p-1");
  visit(
    screen.getByRole("link", { name: /の設定を編集$/ }).getAttribute("href")!,
  );
  await waitFor(() => expect(field("名前").value).toBe(printers[0].name));
  // Secrets are never shown again.
  expect(field("LANアクセスコード").value).toBe("");
  expect(field("TLS証明書（PEM）").value).toBe("");
  await type("名前", "Edited printer");
  await fireEvent.change(field("機種・装着ノズル径"), {
    target: { value: p1s },
  });
  await waitFor(() =>
    expect(field("既定の工程").value).toBe("0.20mm Standard @BBL X1C"),
  );
  // The printer owns its bed.
  await fireEvent.change(field("プレート種類"), {
    target: { value: "High Temp Plate" },
  });
  busy = true;
  await click("保存");
  expect((await screen.findByRole("alert")).textContent).toContain(
    "機器が使用中",
  );
  expect(field("名前").value).toBe("Edited printer");
  busy = false;
  await click("保存");
  await waitFor(() => expect(went).toEqual(["/printers", "/printers"]));
  expect(printers[0]).toMatchObject({
    name: "Edited printer",
    machine_profile_key: p1s,
    bed_type: "High Temp Plate",
  });
  visit("/printers/p-1");
  await waitFor(() => expect(field("名前").value).toBe("Edited printer"));
  await click("削除");
  await waitFor(() => expect(went).toHaveLength(3));
  expect(api.to("DELETE", "/api/printers/p-1")).toHaveLength(1);
  visit("/printers");
  expect(
    await screen.findByText(
      "プリンターを追加すると、接続状態の確認と印刷ができます。",
    ),
  ).toBeTruthy();
});
