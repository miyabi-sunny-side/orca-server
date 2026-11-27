import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, reset, visit, went } from "../testing/page";
import { server } from "../testing/server";

afterEach(reset);

const id = "11111111-1111-4111-8111-111111111111";
const selection = (
  index: number,
  name: string,
  print_reason: string | null = null,
) => ({
  name,
  root_model: "3D/Objects/object_1.model",
  plate_id: String(index + 1),
  items: [{ build_index: index, object_id: index + 1, instance_id: 0 }],
  print_reason,
  roles: ["primary"],
});
const painted =
  "多色・ペイントの印刷は未対応です。元の3MFはそのまま保存できます。";

/** The server reads 3MF files: one bad, one with two plates, one painted; saves fail once. */
function importer() {
  let failSave = true;
  const api = server(({ method, path, body }) => {
    if (path === "/api/plates/file-info") {
      const name = (body as FormData)
        .getAll("models")
        .map((f) => (f as File).name)[0];
      if (name === "bad.3mf")
        return {
          status: 400,
          json: {
            error:
              "3MFの形状または部品参照が不正です。モデルを含むファイルを書き出し直してください。",
          },
        };
      if (name === "painted.3mf")
        return { json: [selection(0, "塗り分け", painted)] };
      if (name === "single.3mf") return { json: [selection(0, "寸法確認")] };
      return { json: [selection(0, "寸法確認 1"), selection(1, "寸法確認 2")] };
    }
    if (path === "/api/plates/file-preview") return { json: [] };
    if (path === "/api/plates/files" && method === "POST") {
      if (failSave) {
        failSave = false;
        return { status: 503, json: { error: "Fixture temporary failure" } };
      }
      return {
        status: 201,
        json: { id, version: 1, name: "saved", models: [] },
      };
    }
    if (path === "/api/default-settings")
      return {
        json: {
          default_printer_id: "p1",
          conditions: { filament_id: "white" },
          infill_patterns: [],
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
  });
  return { api };
}
const file = (name: string) => new File([new Uint8Array([1, 2, 3])], name);
const choose = (...files: File[]) =>
  fireEvent.change(screen.getByLabelText("モデルファイル"), {
    target: { files },
  });
const field = (label: string) =>
  screen.getByLabelText<HTMLInputElement & HTMLSelectElement>(label);
const saved = (api: ReturnType<typeof server>) =>
  api.to("POST", "/api/plates/files").map((r) => r.body as FormData);

it("external models: keep files, plates and edits, and never ask SCAD", async () => {
  const { api } = importer();
  visit("/plates");
  expect(
    (
      await screen.findByRole("link", { name: "ファイルから取り込む" })
    ).getAttribute("href"),
  ).toMatch(/source=file/);
  visit("/plates/new?source=file");
  await waitFor(() =>
    expect(screen.getByLabelText("モデルファイル")).toBeTruthy(),
  );
  await choose(file("bad.3mf"));
  expect((await screen.findByRole("alert")).textContent).toContain("3MF");
  await click("再試行");
  expect(await screen.findByRole("alert")).toBeTruthy();
  // A 3MF of two plates asks which one, and previews that one.
  await choose(file("multiple.3mf"));
  const plate =
    await screen.findByLabelText<HTMLSelectElement>("取り込むプレート");
  await fireEvent.change(plate, { target: { value: "1" } });
  await waitFor(() =>
    expect(
      api.to("POST", "/api/plates/file-preview").at(-1)!.body.get("plate"),
    ).toBe("1"),
  );
  await waitFor(() => expect(field("寸法確認 2 の個数")).toBeTruthy());
  await fireEvent.input(field("プレート名"), {
    target: { value: "外部モデル" },
  });
  await fireEvent.input(field("寸法確認 2 の個数"), { target: { value: "2" } });
  await click("保存");
  await waitFor(() =>
    expect(screen.getAllByRole("alert").some((a) => a.closest("form"))).toBe(
      true,
    ),
  );
  expect(field("プレート名").value).toBe("外部モデル");
  expect(field("寸法確認 2 の個数").value).toBe("2");
  await click("保存");
  await waitFor(() => expect(went).toEqual([`/plates/${id}`]));
  const sent = saved(api)[1];
  expect(sent.getAll("models").map((f) => (f as File).name)).toEqual([
    "multiple.3mf",
  ]);
  expect(String(sent.get("plate"))).toBe("1");
  expect(String(sent.get("name"))).toBe("外部モデル");
  expect(JSON.parse(String(sent.get("quantities")))).toEqual([2]);
  // A painted plate is kept as it is, with its reason.
  visit("/plates/new?source=file");
  await waitFor(() =>
    expect(screen.getByLabelText("モデルファイル")).toBeTruthy(),
  );
  await choose(file("painted.3mf"));
  expect(await screen.findByText(painted)).toBeTruthy();
  // One plate needs no choice.
  await choose(file("single.3mf"));
  await waitFor(() => expect(field("寸法確認 の個数")).toBeTruthy());
  expect(screen.queryByLabelText("取り込むプレート")).toBeNull();
  // STL files are each a model; one can be taken out again.
  await choose(
    file("一つ目のとても長いファイル名を含む形状の確認.stl"),
    file("second.stl"),
  );
  await waitFor(() => expect(field("second.stl の個数")).toBeTruthy());
  expect(field("表示するモデル")).toBeTruthy();
  await fireEvent.input(field("second.stl の個数"), { target: { value: "3" } });
  await click("一つ目のとても長いファイル名を含む形状の確認.stlを構成から外す");
  await waitFor(() =>
    expect(screen.queryByLabelText("表示するモデル")).toBeNull(),
  );
  await click("保存");
  await waitFor(() => expect(went).toHaveLength(2));
  const stls = saved(api).at(-1)!;
  expect(stls.getAll("models").map((f) => (f as File).name)).toEqual([
    "second.stl",
  ]);
  expect(JSON.parse(String(stls.get("quantities")))).toEqual([3]);
  expect(api.sent.filter((r) => r.path.startsWith("/api/scad"))).toEqual([]);
});

it("an imported plate offers its original files and a painted one cannot be queued", async () => {
  const imported = {
    id,
    version: 1,
    name: "外部モデル",
    models: [{ id: "derived", name: "寸法確認 2", source: null, quantity: 2 }],
    conditions: { filament_id: "white" },
    imported: {
      file_name: "multiple.3mf",
      model_id: "derived",
      selection: selection(1, "寸法確認 2"),
    },
  };
  let plate: any = imported;
  server(({ path, url }) => {
    if (path === `/api/plates/${id}`) return { json: plate };
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
    if (path === "/api/queue")
      return {
        json: {
          epoch: "e",
          generation: 0,
          request_id: "r",
          waiting: [],
          current: null,
          admission: {
            allowed: url.searchParams.has("plate_id"),
            plate_version: 1,
            reason: null,
          },
        },
      };
  });
  visit(`/plates/${id}`);
  expect(
    (await screen.findByRole("link", { name: "元の3MFを取得" })).getAttribute(
      "href",
    ),
  ).toBe(`/api/plates/${id}/original`);
  expect(
    screen.getByRole("link", { name: "確認用STLを取得" }).getAttribute("href"),
  ).toBe(`/api/plates/${id}/files/derived`);
  expect(
    screen.queryByRole("link", { name: "アップロードした元STLを取得" }),
  ).toBeNull();
  plate = {
    ...imported,
    imported: {
      ...imported.imported,
      selection: selection(0, "塗り分け", painted),
    },
  };
  visit(`/plates/${id}`);
  await waitFor(() =>
    expect(
      screen.getByRole<HTMLButtonElement>("button", { name: "印刷キューへ" })
        .disabled,
    ).toBe(true),
  );
  expect(screen.getByRole("link", { name: "元の3MFを取得" })).toBeTruthy();
  expect(screen.queryByRole("link", { name: "AMSを確認" })).toBeNull();
  plate = {
    ...imported,
    imported: undefined,
    models: [{ id: "upload", name: "second.stl", source: null, quantity: 3 }],
  };
  visit(`/plates/${id}`);
  expect(
    await screen.findAllByRole("link", { name: "アップロードした元STLを取得" }),
  ).toHaveLength(1);
});
