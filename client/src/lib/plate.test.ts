import { expect, test } from "vitest";
import { machineChoices, destinations, choosePrinter } from "./plate";
const printers = [
  { id: "p1", machine_profile_key: "P1S 0.4" },
  { id: "a3", machine_profile_key: "A1 mini 0.2" },
  { id: "a1", machine_profile_key: "A1 mini 0.2" },
  { id: "a2", machine_profile_key: "A1 mini 0.2" },
];
test("owned profiles deduplicate and only exact machine/nozzle destinations are selectable", () => {
  expect(machineChoices(printers)).toEqual(["A1 mini 0.2", "P1S 0.4"]);
  expect(destinations(printers, "P1S 0.4").map((p) => p.id)).toEqual(["p1"]);
  expect(destinations(printers, "A1 mini 0.2").map((p) => p.id)).toEqual([
    "a1",
    "a2",
    "a3",
  ]);
  expect(destinations(printers, "A1 mini 0.4")).toEqual([]);
  expect(destinations(printers, null)).toEqual([]);
  const choices = destinations(printers, "A1 mini 0.2");
  expect(choosePrinter(choices, "a2")).toBe("a2");
  expect(choosePrinter(choices, "p1")).toBe("a1");
  expect(choosePrinter([], "a2")).toBe("");
});

test("initial defaults fill only untouched blanks and never cross a changed machine", async () => {
  const { initialConditions, emptyConditions } = await import("./plate");
  const defaults = {
    ...emptyConditions,
    required_machine_profile_key: "P1S 0.4",
    filament_id: "white",
    process_profile_key: "standard",
    bed_type: "Cool Plate",
  };
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
  expect(
    initialConditions(
      { ...emptyConditions, required_machine_profile_key: "A1 mini 0.2" },
      defaults,
      new Set(["required_machine_profile_key"]),
    ),
  ).toEqual({
    ...emptyConditions,
    required_machine_profile_key: "A1 mini 0.2",
    bed_type: "Cool Plate",
  });
});

test("strength defaults are independent of machine, preserve manual values and never backfill old plates", async () => {
  const { initialConditions, emptyConditions } = await import("./plate");
  const defaults = {
    ...emptyConditions,
    required_machine_profile_key: "P1S",
    sparse_infill_pattern: "adaptivecubic",
    sparse_infill_density: 15,
    wall_loops: 2,
  };
  const manual = {
    ...emptyConditions,
    required_machine_profile_key: "A1",
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
