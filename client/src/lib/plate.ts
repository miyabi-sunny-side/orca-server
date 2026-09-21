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
