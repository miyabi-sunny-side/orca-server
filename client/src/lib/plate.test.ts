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
