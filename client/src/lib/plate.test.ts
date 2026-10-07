import { expect, test } from "vitest";
import {
  destinations,
  choosePrinter,
  chooseModels,
  materialRoles,
} from "./plate";
test("only roles actually used by the composition appear, preserving monochrome compatibility", () => {
  expect(materialRoles([{}])).toEqual(["primary"]);
  expect(materialRoles([{ roles: ["secondary"] }])).toEqual(["secondary"]);
  expect(
    materialRoles([
      { roles: ["secondary", "primary"] },
      { roles: ["primary"] },
    ]),
  ).toEqual(["primary", "secondary"]);
  expect(materialRoles([])).toEqual([]);
});
test("catalog additions deduplicate and replacement preserves quantity and position without the old ID", () => {
  const models = [
    { id: "upload", name: "original.stl", source: null, quantity: 10 },
    { id: "reference", name: "other.stl", source: "other.stl", quantity: 3 },
  ];
  expect(chooseModels(models, ["new.stl", "other.stl", "new.stl"])).toEqual([
    ...models,
    { name: "new.stl", source: "new.stl", quantity: 1 },
  ]);
  expect(chooseModels(models, ["new.stl"], 0)).toEqual([
    { name: "new.stl", source: "new.stl", quantity: 10 },
    models[1],
  ]);
  expect(chooseModels(models, ["other.stl"], 0)).toEqual(models);
  expect(chooseModels(models, [], 0)).toEqual(models);
  expect(models[0].id).toBe("upload");
  expect(chooseModels([], ["new.stl"])).toEqual([
    { name: "new.stl", source: "new.stl", quantity: 1 },
  ]);
});
const printers = [
  { id: "p1", machine_profile_key: "P1S 0.4" },
  { id: "a3", machine_profile_key: "A1 mini 0.2" },
  { id: "a1", machine_profile_key: "A1 mini 0.2" },
  { id: "a2", machine_profile_key: "A1 mini 0.2" },
];
test("editing never backfills saved nullable conditions with creation defaults", async () => {
  const { initialConditions, emptyConditions } = await import("./plate");
  expect(
    initialConditions(
      emptyConditions,
      { ...emptyConditions, filament_id: "white", wall_loops: 2 },
      new Set(),
      false,
    ),
  ).toEqual(emptyConditions);
});
test("every registered printer is a destination; the server decides whether the plate fits it", () => {
  const choices = destinations(printers);
  expect(choices.map((p) => p.id)).toEqual(["a1", "a2", "a3", "p1"]);
  expect(choosePrinter(choices, "a2")).toBe("a2");
  expect(choosePrinter(choices, "missing")).toBe("a1");
  expect(choosePrinter([], "a2")).toBe("");
});

test("initial defaults fill only untouched blanks", async () => {
  const { initialConditions, emptyConditions } = await import("./plate");
  const defaults = { ...emptyConditions, filament_id: "white" };
  expect(initialConditions(emptyConditions, defaults, new Set())).toEqual(
    defaults,
  );
  expect(
    initialConditions(
      { ...emptyConditions, filament_id: "blue" },
      defaults,
      new Set(),
    ),
  ).toEqual({ ...defaults, filament_id: "blue" });
  expect(
    initialConditions(emptyConditions, defaults, new Set(["filament_id"])),
  ).toEqual({ ...defaults, filament_id: null });
  expect(Object.keys(emptyConditions)).not.toContain("bed_type");
});

test("strength defaults preserve manual values and never backfill old plates", async () => {
  const { initialConditions, emptyConditions } = await import("./plate");
  const defaults = {
    ...emptyConditions,
    sparse_infill_pattern: "adaptivecubic",
    sparse_infill_density: 15,
    wall_loops: 2,
  };
  const manual = {
    ...emptyConditions,
    wall_loops: 4,
    sparse_infill_density: 0,
  };
  expect(initialConditions(manual, defaults, new Set())).toMatchObject({
    sparse_infill_pattern: "adaptivecubic",
    sparse_infill_density: 0,
    wall_loops: 4,
  });
  expect(
    initialConditions(
      emptyConditions,
      defaults,
      new Set(["sparse_infill_pattern"]),
    ),
  ).toMatchObject({
    sparse_infill_pattern: null,
    sparse_infill_density: 15,
    wall_loops: 2,
  });
  expect(
    initialConditions(emptyConditions, defaults, new Set(), false),
  ).toMatchObject({
    sparse_infill_pattern: null,
    sparse_infill_density: null,
    wall_loops: null,
  });
});

test("old forms default brim off and late defaults preserve the user's checkbox", async () => {
  const { initialConditions, emptyConditions } = await import("./plate");
  expect(
    initialConditions(emptyConditions, emptyConditions, new Set()),
  ).toHaveProperty("brim_enabled", false);
  const checked = { ...emptyConditions, brim_enabled: true };
  expect(
    initialConditions(checked, emptyConditions, new Set(["brim_enabled"])),
  ).toHaveProperty("brim_enabled", true);
  expect(
    initialConditions({ ...checked, brim_enabled: false }, checked, new Set()),
  ).toHaveProperty("brim_enabled", false);
});

test("support starts off, defaults its interface once, and preserves a chosen material", async () => {
  const { initialConditions, emptyConditions } = await import("./plate");
  const defaults = { ...emptyConditions, filament_id: "white" };
  expect(initialConditions(emptyConditions, defaults, new Set())).toMatchObject(
    {
      support_enabled: false,
      support_interface_filament_id: null,
    },
  );
  expect(
    initialConditions(
      { ...defaults, support_enabled: true },
      defaults,
      new Set(),
    ),
  ).toMatchObject({
    support_enabled: true,
    support_interface_filament_id: "white",
  });
  for (const support_enabled of [false, true]) {
    expect(
      initialConditions(
        { ...defaults, support_enabled, support_interface_filament_id: "petg" },
        defaults,
        new Set(),
      ),
    ).toMatchObject({
      support_enabled,
      support_interface_filament_id: "petg",
    });
  }
  expect(
    initialConditions(
      { ...emptyConditions, support_enabled: true },
      emptyConditions,
      new Set(),
    ),
  ).toMatchObject({ support_interface_filament_id: null });
});
