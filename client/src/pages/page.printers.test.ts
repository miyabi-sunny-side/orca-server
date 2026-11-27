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
