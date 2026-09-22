import type { ImportedSelection } from "./api";
export type UploadDraft = {
  files: File[];
  plate?: number;
  selection?: ImportedSelection;
  previews: string[];
};
export function uploadBody(files: File[], plate?: number): FormData {
  const body = new FormData();
  for (const file of files) body.append("models", file);
  if (plate !== undefined) body.append("plate", String(plate));
  return body;
}
