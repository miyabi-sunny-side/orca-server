import { expect, test, type Page } from "@playwright/test";
import { writeFileSync } from "node:fs";

/** Visible text and controls of the main area, as a reader meets them on arrival. */
async function measure(page: Page) {
  return page.evaluate(() => {
    const main = document.querySelector("main")!;
    const visible = (e: Element) => {
      const r = e.getBoundingClientRect();
      return (
        r.width > 0 &&
        r.height > 0 &&
        !e.closest("details:not([open]) > :not(summary)")
      );
    };
    const text = main.innerText.replace(/\s+/g, "");
    const controls = [
      ...main.querySelectorAll("button, a.btn, [role=button]"),
    ].filter(visible);
    const labelled = controls.map((c) => (c.textContent ?? "").trim());
    return {
      characters: text.length,
      paragraphs: [...main.querySelectorAll("p")].filter(
        (p) => visible(p) && (p.textContent ?? "").trim().length > 40,
      ).length,
      controls: controls.length,
      long_labels: labelled.filter((t) => t.length > 8),
      icon_controls: labelled.filter((t) => t.length === 0).length,
      unnamed: [...main.querySelectorAll("button, a")]
        .filter(visible)
        .filter(
          (c) =>
            !(c.textContent ?? "").trim() &&
            !c.getAttribute("aria-label") &&
            !c.getAttribute("title"),
        ).length,
      height: document.documentElement.scrollHeight,
    };
  });
}

test("every main screen is measured with the same data", async ({
  page,
  request,
}) => {
  test.setTimeout(300_000);
  const context = JSON.parse(process.env.E2E_UI_AUDIT_CONTEXT!);
  const phase = process.env.E2E_UI_PHASE ?? "after";
  const products = await (await request.get("/api/filament-products")).json();
  const screens: [string, string][] = [
    ["queue", "/"],
    ["plates", "/plates"],
    ["plate", `/plates/${context.plate}`],
    ["plate-edit", `/plates/${context.plate}?edit=1`],
    ["plate-new", "/plates/new"],
    ["printers", "/printers"],
    ["printer-settings", "/printers/p1"],
    ["ams", "/printers/p1/ams"],
    ["control", "/printers/p1/control"],
    ["filaments", "/filaments"],
    ["filament", `/filaments/${products[0].id}`],
    ["history", "/history"],
    ["about", "/about"],
  ];
  const results: Record<string, unknown> = {};
  for (const [width, height, scheme] of [
    [1280, 900, "light"],
    [375, 812, "dark"],
  ] as const) {
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ colorScheme: scheme });
    for (const [name, path] of screens) {
      await page.goto(path);
      await page.waitForLoadState("networkidle");
      await page.waitForTimeout(500);
      results[`${name}@${width}`] = await measure(page);
      if (phase === "after") {
        const m = results[`${name}@${width}`] as { unnamed: number };
        expect(m.unnamed, `${name}@${width}: unnamed icon controls`).toBe(0);
        // Text at 200%: every screen still fits the width.
        await page.addStyleTag({
          content: "html { font-size: 200% !important; }",
        });
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth + 1,
          ),
          `${name}@${width}: 200% text overflows`,
        ).toBe(true);
      }
      await page.screenshot({
        path: `${process.env.E2E_EVIDENCE_DIR}/${phase}-${name}-${width}-${scheme}.png`,
        fullPage: true,
      });
    }
  }
  writeFileSync(
    `${process.env.E2E_EVIDENCE_DIR}/ui-metrics-${phase}.json`,
    JSON.stringify(results, null, 2),
  );
});
