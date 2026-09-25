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
      conditions: {
        required_machine_profile_key: "P1S 0.4",
        filament_id: "pla",
        process_profile_key: "Standard",
        bed_type: "Textured PEI Plate",
      },
    };
    let state = {
        state: "calculating",
        seconds: null as number | null,
        error: null as string | null,
      },
      retries = 0;
    await page.route("**/api/**", (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path.endsWith("/slice")) {
        if (route.request().method() === "POST") {
          retries++;
          state = { state: "calculating", seconds: null, error: null };
          return route.fulfill({ status: 202 });
        }
        return route.fulfill({ json: state });
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
