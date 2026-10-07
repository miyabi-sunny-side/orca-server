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
type Device = { id: string };
/** Every printer can be chosen; its own slice result decides whether the plate fits. */
export function destinations<T extends Device>(printers: T[]) {
  return [...printers].sort((a, b) => a.id.localeCompare(b.id));
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
  filament_id: null,
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
    if (result[key] == null && !edited.has(key))
      Object.assign(result, { [key]: defaults[key] ?? null });
  }
  if (result.support_enabled)
    result.support_interface_filament_id ??= result.filament_id;
  return result;
}
