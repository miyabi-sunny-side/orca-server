import { expect, test } from "@playwright/test";

for (const width of [320, 375, 900]) {
  for (const colorScheme of ["dark", "light"] as const) {
    test(`${width}px ${colorScheme}: connection and theme controls`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ colorScheme });
      await page.route("**/api/health", (route) =>
        route.fulfill({ json: { status: "ok" } }),
      );
      await page.goto("/");
      await expect(page.getByRole("status")).toHaveText(
        "OrcaServerに接続しました",
      );
      await expect(page).toHaveTitle("OrcaServer");
      await expect(page.locator("header a, header button")).toHaveCount(2);
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      expect((await page.getByRole("status").boundingBox())!.y).toBeLessThan(
        120,
      );
      await expect(page.locator("body")).toHaveCSS(
        "background-color",
        colorScheme === "dark" ? "rgb(25, 25, 25)" : "rgb(250, 246, 239)",
      );
      await page.getByRole("button", { name: "メニュー", exact: true }).focus();
      await page.keyboard.press("Enter");
      await page
        .getByRole("button", { name: "テーマ設定", exact: true })
        .click();
      const dialog = page.getByRole("dialog", { name: "テーマ設定" });
      await expect(dialog).toBeVisible();
      await page.getByRole("radio", { name: "ライト", exact: true }).click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
      await expect(dialog).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(
        page.getByRole("button", { name: "メニュー", exact: true }),
      ).toBeFocused();
      await page.reload();
      await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
      await page.getByRole("button", { name: "メニュー", exact: true }).click();
      await page
        .getByRole("button", { name: "テーマ設定", exact: true })
        .click();
      await page.getByRole("radio", { name: "自動", exact: true }).click();
      expect(
        await page.evaluate(() => localStorage.getItem("orca-server:theme")),
      ).toBeNull();
      await expect(page.locator("html")).not.toHaveAttribute("data-theme");
      await page.keyboard.press("Escape");
      if (process.env.E2E_EVIDENCE_DIR) {
        await page.screenshot({
          path: `${process.env.E2E_EVIDENCE_DIR}/home-${width}-${colorScheme}.png`,
        });
      }
    });
  }
}

test("loading and failed connection recover through retry", async ({
  page,
}) => {
  let finish!: () => void;
  let fail = true;
  await page.route("**/api/health", async (route) => {
    if (fail) {
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      await route.fulfill({ status: 503, json: { error: "unavailable" } });
    } else await route.fulfill({ json: { status: "ok" } });
  });
  await page.goto("/");
  await expect(page.getByRole("status")).toContainText("接続を確認しています");
  await expect.poll(() => typeof finish).toBe("function");
  finish();
  await expect(page.getByRole("alert")).toHaveText("接続できませんでした");
  fail = false;
  await page.getByRole("button", { name: "再試行" }).click();
  await expect(page.getByRole("status")).toHaveText("OrcaServerに接続しました");
});
