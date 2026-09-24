import type { MaterialRole, PlateConditions } from "./api";
export type EditModel = {
  id?: string;
  fileIndex?: number;
  name: string;
  source: string | null;
  quantity: number;
  roles?: MaterialRole[];
};
export const roleFields = {
  primary: "filament_id",
  secondary: "secondary_filament_id",
} as const;
export function materialRoles(
  models: { roles?: MaterialRole[] }[],
): MaterialRole[] {
  const used = new Set(
    models.flatMap((m) => (m.roles?.length ? m.roles : ["primary"])),
  );
  return (["primary", "secondary"] as MaterialRole[]).filter((role) =>
    used.has(role),
  );
}

export function chooseModels(
  models: EditModel[],
  sources: string[],
  replacement: number | null = null,
): EditModel[] {
  const additions = [...new Set(sources)].filter(
    (source) => !models.some((model) => model.source === source),
  );
  if (replacement !== null) {
    const source = additions[0];
    return source
      ? models.map((model, index) =>
          index === replacement
            ? { name: source, source, quantity: model.quantity }
            : model,
        )
      : models;
  }
  return [
    ...models,
    ...additions.map((source) => ({ name: source, source, quantity: 1 })),
  ];
}
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
export const emptyStrength = {
  sparse_infill_pattern: null,
  sparse_infill_density: null,
  wall_loops: null,
};
export const emptyConditions: PlateConditions = {
  brim_enabled: false,
  support_enabled: false,
  support_interface_filament_id: null,
  ...emptyStrength,
  required_machine_profile_key: null,
  filament_id: null,
  process_profile_key: null,
  bed_type: null,
};

export function initialConditions(
  value: PlateConditions,
  defaults: PlateConditions,
  edited: Set<keyof PlateConditions>,
  creation = true,
): PlateConditions {
  const result = {
    ...value,
    brim_enabled: value.brim_enabled ?? false,
    support_enabled: value.support_enabled ?? false,
    support_interface_filament_id: value.support_interface_filament_id ?? null,
  };
  if (!creation) return result;
  for (const key of Object.keys(emptyConditions) as (keyof PlateConditions)[]) {
    const strength = key in emptyStrength;
    if (
      result[key] == null &&
      !edited.has(key) &&
      (strength ||
        key === "required_machine_profile_key" ||
        key === "bed_type" ||
        result.required_machine_profile_key ===
          defaults.required_machine_profile_key)
    )
      Object.assign(result, { [key]: defaults[key] ?? null });
  }
  if (result.support_enabled)
    result.support_interface_filament_id ??= result.filament_id;
  return result;
}
