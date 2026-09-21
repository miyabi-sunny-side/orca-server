import { expect, test } from "@playwright/test";

test.beforeEach(async ({page})=>{await page.route("**/api/printers",route=>route.fulfill({json:[]})); await page.route("**/api/filaments",route=>route.fulfill({json:[]})); await page.route("**/api/default-settings*",route=>route.fulfill({json:{"default_printer_id":null,"conditions":{"required_machine_profile_key":null,"filament_id":null,"process_profile_key":null,"bed_type":null},"reason":"printer"}}));});

const plates = Array.from({ length: 100 }, (_, i) => ({
  id: `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
  version: 1,
  name:
    i % 3 === 0
      ? `机の配線整理・ケーブルホルダー ${i + 1}`
      : `小物ケース ${i + 1}`,
  models: [{ id: "item", name: "box.stl", source: "box.stl", quantity: 1 }],
  settings: {},
  project: "project.3mf",
  print: "print.gcode.3mf",
}));
for (const width of [320, 375, 900]) {
  for (const colorScheme of ["dark", "light"] as const) {
    test(`${width}px ${colorScheme}: searchable list and theme`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 812 });
      await page.emulateMedia({ colorScheme });
      await page.route("**/api/plates?*", (route) => {
        const q = new URL(route.request().url()).searchParams.get("q");
        return route.fulfill({
          json: q ? plates.filter((p) => p.name.includes(q)) : plates,
        });
      });
      await page.goto("/plates");
      await expect(page.locator(".plate-row")).toHaveCount(100);
      await expect(page).toHaveTitle("OrcaServer");
      await expect(page.locator("header a, header button")).toHaveCount(3);
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      expect(
        (await page.locator(".plate-row").first().boundingBox())!.y,
      ).toBeLessThanOrEqual(210);
      const visible = await page
        .locator(".plate-row")
        .evaluateAll(
          (rows) =>
            rows.filter(
              (row) => row.getBoundingClientRect().bottom <= innerHeight,
            ).length,
        );
      if (width === 375) expect(visible).toBeGreaterThanOrEqual(6);
      expect(
        await page
          .locator(".plate-list")
          .evaluate((el) => getComputedStyle(el).overflowY),
      ).toBe("visible");
      await expect(page.locator("body")).toHaveCSS(
        "background-color",
        colorScheme === "dark" ? "rgb(25, 25, 25)" : "rgb(250, 246, 239)",
      );
      expect(
        (await page
          .getByRole("button", { name: "メニュー", exact: true })
          .boundingBox())!.width,
      ).toBe(36);
      const search = page.getByRole("searchbox");
      await search.focus();
      await page.keyboard.press("ArrowDown");
      await expect(page.locator(".plate-row").first()).toBeFocused();
      await page.keyboard.press("ArrowDown");
      await expect(page.locator(".plate-row").nth(1)).toBeFocused();
      await search.fill("小物ケース 20");
      await expect(page.locator(".plate-row")).toHaveCount(1);
      await search.fill("");
      await expect(page.locator(".plate-row")).toHaveCount(100);
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
      await expect(page.locator(".plate-row")).toHaveCount(100);
      if (process.env.E2E_EVIDENCE_DIR)
        await page.screenshot({
          path: `${process.env.E2E_EVIDENCE_DIR}/home-${width}-${colorScheme}.png`,
        });
      // Double text sizes, including token-based fixed sizes, without reducing the viewport.
      await page.addStyleTag({
        content:
          ":root { --fs-xs:24px; --fs-sm:28px; --fs-md:30px; --fs-lg:32px; --fs-xl:34px; }",
      });
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      await expect(page.getByRole("link", { name: "新規作成" })).toBeVisible();
      if (width === 375 && process.env.E2E_EVIDENCE_DIR)
        await page.screenshot({
          path: `${process.env.E2E_EVIDENCE_DIR}/text-200-${colorScheme}.png`,
        });
    });
  }
}

test("loading and failed list recover through retry", async ({ page }) => {
  let finish!: () => void;
  let fail = true;
  await page.route("**/api/plates?*", async (route) => {
    if (fail) {
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
      await route.fulfill({ status: 500, json: { error: "unavailable" } });
    } else await route.fulfill({ json: [] });
  });
  await page.goto("/plates");
  await expect(page.getByRole("status")).toContainText("読み込んでいます");
  await expect.poll(() => typeof finish).toBe("function");
  finish();
  await expect(page.getByRole("alert")).toContainText("処理に失敗しました");
  fail = false;
  await page.getByRole("button", { name: "再試行", exact: true }).click();
  await expect(page.getByText("保存済みプレートはありません")).toBeVisible();
});

test("keyboard selection persists across filtering and failed saves retain edits", async ({ page }) => {
  let saves = 0;
  const bodies: { models: { source: string; quantity: number }[] }[] = [];
  await page.route("**/api/scad/models?*", route => {
    const q = new URL(route.request().url()).searchParams.get("q") || "";
    return route.fulfill({ json: ["box.stl", "holder.stl"].filter(name => name.includes(q)) });
  });
  await page.route("**/api/plates/import", route => {
    saves++; bodies.push(route.request().postDataJSON());
    return route.fulfill({ status: 400, json: { error: "bad composition" } });
  });
  await page.goto("/plates/new");
  await expect(page.getByRole("checkbox")).toHaveCount(2);
  await page.getByRole("searchbox").focus(); await page.keyboard.press("ArrowDown"); await page.keyboard.press("Space");
  await expect(page.getByRole("checkbox", { name: "box.stl" })).toBeChecked();
  await page.getByRole("searchbox").fill("holder"); await expect(page.getByRole("checkbox")).toHaveCount(1);
  await page.getByRole("checkbox").check(); await page.getByRole("button", { name: "構成を確認（2）" }).click();
  await page.getByLabel("プレート名", { exact: true }).fill("小物入れ");
  await page.getByLabel("box.stl の個数").fill("3");
  await page.getByRole("button", { name: "保存", exact: true }).click(); await expect(page.getByRole("alert")).toBeVisible();
  await page.getByRole("button", { name: "モデル選択へ" }).click();
  await page.getByRole("checkbox", { name: "holder.stl" }).uncheck(); await page.getByRole("button", { name: "構成を確認（1）" }).click();
  await expect(page.getByLabel("プレート名", { exact: true })).toHaveValue("小物入れ");
  await expect(page.getByLabel("box.stl の個数")).toHaveValue("3");
  await page.getByRole("button", { name: "保存", exact: true }).click(); await expect.poll(() => saves).toBe(2);
  expect(bodies[1].models).toEqual([{ name: "box.stl", source: "box.stl", quantity: 3 }]);
});

test("unconfigured services expose recovery without hiding the plate list", async ({
  page,
}) => {
  await page.route("**/api/slicer/profiles", (route) =>
    route.fulfill({ status: 503, json: { error: "unset" } }),
  );
  await page.route("**/api/scad/models?*", (route) =>
    route.fulfill({ status: 503, json: { error: "unset" } }),
  );
  await page.goto("/plates/new");
  await expect(page.getByRole("alert")).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: "構成を確認（0）" }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "再試行" }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "プレート一覧へ" }),
  ).toHaveAttribute("href", "/plates");
});
