import { screen, waitFor } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { reset, visit } from "../testing/page";
import answers from "../testing/screens.json";
import { server } from "../testing/server";

afterEach(reset);

/** One printer with a failed job, and what each main screen reads (screens.json, shared with tests/page.rs). */
function everything() {
  const known = answers as Record<string, unknown>;
  server(({ path }) =>
    path in known ? { json: structuredClone(known[path]) } : undefined,
  );
}
const plate = "/plates/11111111-1111-4111-8111-111111111111";
const screens: [string, string, string][] = [
  ["queue", "/", ".current-job"],
  ["plates", "/plates", ".plate-row"],
  ["plate", plate, "h1"],
  ["plate-edit", `${plate}?edit=1`, "form"],
  ["plate-new", "/plates/new", "input[type=checkbox]"],
  ["printers", "/printers", "select"],
  ["printer-settings", "/printers/p1", "form"],
  ["ams", "/printers/p1/ams", ".ams-slot"],
  ["control", "/printers/p1/control", "details"],
  ["filaments", "/filaments", "a[href='/filaments/product']"],
  ["filament", "/filaments/product", "h1"],
  ["history", "/history", ".history-row"],
  ["about", "/about", "h1"],
];
/** Not inside a closed disclosure, where the browser draws nothing. */
const shown = (element: Element) =>
  !element.closest("[hidden]") &&
  !(element.closest("details:not([open])") && !element.closest("summary"));

it.each(screens)(
  "%s names every control a reader meets",
  async (_, path, ready) => {
    everything();
    visit(path);
    await waitFor(() =>
      expect(document.querySelector(`main ${ready}`)).toBeTruthy(),
    );
    const unnamed = [...screen.getByRole("main").querySelectorAll("button, a")]
      .filter(shown)
      .filter(
        (c) =>
          !(c.textContent ?? "").trim() &&
          !c.getAttribute("aria-label") &&
          !c.getAttribute("title"),
      )
      .map((c) => c.outerHTML);
    expect(unnamed).toEqual([]);
  },
);
