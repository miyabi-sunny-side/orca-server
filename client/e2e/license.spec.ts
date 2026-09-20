import { expect, test } from "@playwright/test";

for (const colorScheme of ["dark", "light"] as const) {
  test(`license menu and source recovery in ${colorScheme}`, async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 812 });
    await page.emulateMedia({ colorScheme });
    let failed = true;
    let source: string | null = "https://example.org/source.tar.gz";
    await page.route("**/api/plates?*", route => route.fulfill({ json: [] }));
    await page.route("**/api/about", route => failed
      ? route.fulfill({ status: 500, json: { error: "unavailable" } })
      : route.fulfill({ json: { version: "0.1.8", source_url: source } }));
    await page.goto("/");
    await page.getByRole("button", { name: "メニュー", exact: true }).click();
    const link = page.getByRole("link", { name: "ライセンスとソース" });
    await link.focus();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/about$/);
    await expect(page.getByRole("alert")).toBeVisible();
    failed = false;
    await page.getByRole("button", { name: "再試行" }).click();
    await expect(page.getByRole("heading", { name: "OrcaServer 0.1.8" })).toBeVisible();
    await expect(page.getByRole("link", { name: "このビルドのソースを取得" })).toHaveAttribute("href", source!);
    await expect(page.getByRole("link", { name: "ライセンス本文" })).toHaveAttribute("href", "/LICENSE");
    await expect(page.getByRole("link", { name: "第三者の著作権・許諾表示" })).toHaveAttribute("href", "/THIRD_PARTY_NOTICES");
    await expect(page.locator("body")).toHaveCSS("background-color", colorScheme === "dark" ? "rgb(25, 25, 25)" : "rgb(250, 246, 239)");
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    if (process.env.E2E_EVIDENCE_DIR) await page.screenshot({ path: `${process.env.E2E_EVIDENCE_DIR}/license-320-${colorScheme}.png`, fullPage: true });
    source = null;
    await page.reload();
    await expect(page.getByText(/公開先が設定されていません/)).toBeVisible();
    await expect(page.getByRole("link", { name: "このビルドのソースを取得" })).toHaveCount(0);
    await page.getByRole("link", { name: "プレート一覧へ" }).click();
    await expect(page.getByText("保存済みプレートはありません")).toBeVisible();
  });
}
