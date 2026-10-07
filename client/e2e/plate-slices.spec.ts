import { test, expect } from "@playwright/test";
const id = "11111111-1111-4111-8111-111111111111";
for (const colorScheme of ["dark", "light"] as const) {
  test(`plate calculation states and recovery at narrow width ${colorScheme}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.emulateMedia({ colorScheme });
    const plate = {
      id,
      version: 1,
      name: "キューブの保存済みプレート",
      models: [
        { id: "item", name: "cube.stl", source: "cube.stl", quantity: 2 },
      ],
      conditions: { filament_id: "pla" },
    };
    let state: Record<string, unknown> = {
        state: "calculating",
        seconds: null as number | null,
        error: null as string | null,
      },
      mini: Record<string, unknown> | null = null,
      retries = 0;
    const p1 = {
      printer_id: "p1",
      printer_name: "P1S",
      machine_profile_key: "P1S 0.4",
    };
    await page.route("**/api/**", (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path.endsWith("/slice")) {
        if (route.request().method() === "POST") {
          retries++;
          state = { state: "calculating", seconds: null, error: null };
          return route.fulfill({ status: 202 });
        }
        return route.fulfill({
          json: {
            printers: [
              ...(mini
                ? [
                    {
                      printer_id: "a1",
                      printer_name: "A1 mini",
                      machine_profile_key: "A1 mini 0.4",
                      ...mini,
                    },
                  ]
                : []),
              { ...p1, ...state },
            ],
          },
        });
      }
      if (path === `/api/plates/${id}`) return route.fulfill({ json: plate });
      if (path === "/api/filaments")
        return route.fulfill({ json: [{ id: "pla", name: "PLA 青" }] });
      return route.fulfill({ json: [] });
    });
    await page.goto(`/plates/${id}`);
    await expect(page.getByRole("heading", { name: plate.name })).toBeVisible();
    await expect(page.getByLabel("プレートの試算")).toContainText("試算中…");
    state = { state: "ready", seconds: 1140, error: null };
    await expect(page.getByLabel("プレートの試算")).toContainText("約19分");
    state = {
      state: "failed",
      seconds: null,
      error:
        "Selected build plate temperature is missing or zero for this material",
    };
    const status = page.getByLabel("プレートの試算");
    await expect(status).toContainText("試算できませんでした");
    await status.locator("summary").click();
    await expect(
      status.getByRole("link", { name: /材料の設定/ }),
    ).toHaveAttribute("href", /\/filaments\/pla/);
    await status.getByRole("button", { name: "再試算", exact: true }).click();
    await expect.poll(() => retries).toBe(1);
    await expect(status).toContainText("試算中…");
    state = { state: "ready", seconds: 1140, error: null };
    await expect(status).toContainText("約19分");
    // One printer keeps the single line; it does not name the printer.
    await expect(status).not.toContainText("P1S");
    await page.screenshot({
      path: testInfo.outputPath(`one-printer-${colorScheme}.png`),
      fullPage: true,
    });
    // With a second printer each line names its printer and why it cannot print.
    mini = {
      state: "failed",
      seconds: null,
      error: "Models must fit together on one plate",
      reason: "unfit",
    };
    const rows = status.getByRole("listitem");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toContainText("A1 mini · 台に乗りません");
    await expect(rows.nth(1)).toContainText("P1S · 約19分");
    await rows.nth(0).locator("summary").click();
    await expect(rows.nth(0).getByRole("alert")).toContainText("台に乗りません");
    await expect(rows.nth(0).getByRole("link", { name: /材料の設定/ })).toHaveCount(0);
    mini = {
      state: "failed",
      seconds: null,
      error: "Configure this material for the required machine and nozzle first",
      reason: "material_setting",
    };
    await expect(rows.nth(0)).toContainText("A1 mini · 材料設定がありません");
    await expect(
      rows.nth(0).getByRole("link", { name: /材料の設定/ }),
    ).toHaveAttribute("href", /\/filaments\/pla\?machine=A1%20mini%200\.4/);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(`after-${colorScheme}.png`),
      fullPage: true,
    });
  });
}
