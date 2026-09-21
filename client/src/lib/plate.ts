import type { PlateConditions } from "./api";
type Device = { id: string; machine_profile_key: string };
export function machineChoices(printers: Device[]) {
  return [...new Set(printers.map((p) => p.machine_profile_key))].sort();
}
export function destinations<T extends Device>(
  printers: T[],
  machine: string | null,
) {
  return printers
    .filter((p) => p.machine_profile_key === machine)
    .sort((a, b) => a.id.localeCompare(b.id));
}
export function choosePrinter(printers: Device[], previous: string) {
  return printers.some((p) => p.id === previous)
    ? previous
    : (printers[0]?.id ?? "");
}
export const emptyConditions: PlateConditions = {
  required_machine_profile_key: null,
  filament_id: null,
  process_profile_key: null,
  bed_type: null,
};

export function initialConditions(
  value: PlateConditions,
  defaults: PlateConditions,
  edited: Set<keyof PlateConditions>,
): PlateConditions {
  const result = { ...value };
  for (const key of Object.keys(emptyConditions) as (keyof PlateConditions)[]) {
    if (
      result[key] === null &&
      !edited.has(key) &&
      (key === "required_machine_profile_key" ||
        key === "bed_type" ||
        result.required_machine_profile_key ===
          defaults.required_machine_profile_key)
    )
      result[key] = defaults[key];
  }
  return result;
}
