import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { reload, reset, visit } from "../testing/page";
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
