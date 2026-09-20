import { expect, test } from "@playwright/test";

test.beforeEach(async ({page})=>{await page.route("**/api/printers",route=>route.fulfill({json:[]}));});

const plates = Array.from({ length: 100 }, (_, i) => ({
  id: `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
  revision: "revision-1",
  name:
    i % 3 === 0
      ? `机の配線整理・ケーブルホルダー ${i + 1}`
      : `小物ケース ${i + 1}`,
  models: [{ name: "box.stl", source: "box.stl" }],
  settings: {},
  project: "project.3mf",
  print: "print.gcode.3mf",
}));
const profiles = {
  printer: "Bambu Lab P1S 0.4 nozzle",
  version: "2.4.2",
  processes: ["0.20mm Standard @BBL X1C"],
  filaments: ["Generic PLA @BBL X1C"],
  beds: ["Textured PEI Plate"],
  defaults: {
    process: "0.20mm Standard @BBL X1C",
    filament: "Generic PLA @BBL X1C",
    bed: "Textured PEI Plate",
  },
};

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
      await page.goto("/");
      await expect(page.locator(".plate-row")).toHaveCount(100);
      await expect(page).toHaveTitle("OrcaServer");
      await expect(page.locator("header a, header button")).toHaveCount(2);
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
  await page.goto("/");
  await expect(page.getByRole("status")).toContainText("読み込んでいます");
  await expect.poll(() => typeof finish).toBe("function");
  finish();
  await expect(page.getByRole("alert")).toContainText("処理に失敗しました");
  fail = false;
  await page.getByRole("button", { name: "再試行", exact: true }).click();
  await expect(page.getByText("保存済みプレートはありません")).toBeVisible();
});

test("keyboard selection persists across filtering; failed slice retries or edits the same plate", async ({
  page,
}) => {
  let imports = 0;
  let slices = 0;
  const bodies: { models: string[]; plate_id?: string }[] = [];
  await page.route("**/api/slicer/profiles", (route) =>
    route.fulfill({ json: profiles }),
  );
  await page.route("**/api/scad/models?*", (route) => {
    const q = new URL(route.request().url()).searchParams.get("q") || "";
    return route.fulfill({
      json: ["box.stl", "holder.stl"].filter((name) => name.includes(q)),
    });
  });
  await page.route("**/api/plates/import", (route) => {
    imports++;
    bodies.push(route.request().postDataJSON());
    return route.fulfill({ status: 201, json: plates[0] });
  });
  await page.route("**/api/plates/*/slice", (route) => {
    slices++;
    return route.fulfill({ status: 502, json: { error: "exit" } });
  });
  await page.goto("/plates/new");
  await expect(page.getByRole("checkbox")).toHaveCount(2);
  await page.getByRole("searchbox").focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Space");
  await expect(page.getByRole("checkbox", { name: "box.stl" })).toBeChecked();
  await page.getByRole("searchbox").fill("holder");
  await expect(page.getByRole("checkbox")).toHaveCount(1);
  await page.getByRole("checkbox").check();
  await page.getByRole("button", { name: "設定へ（2）" }).click();
  await page.getByRole("textbox", { name: "プレート名" }).fill("小物入れ");
  await page.getByRole("button", { name: "配置して保存" }).click();
  await expect(page.getByRole("alert")).toContainText("失敗しました");
  await page.getByRole("button", { name: "もう一度配置する" }).click();
  await expect.poll(() => slices).toBe(2);
  expect(imports).toBe(1);
  await page.getByRole("button", { name: "選択へ戻る" }).click();
  await page.getByRole("checkbox", { name: "holder.stl" }).uncheck();
  await page.getByRole("button", { name: "設定へ（1）" }).click();
  await expect(page.getByRole("textbox", { name: "プレート名" })).toHaveValue(
    "小物入れ",
  );
  await page.getByRole("button", { name: "もう一度配置する" }).click();
  await expect.poll(() => slices).toBe(3);
  expect(imports).toBe(2);
  expect(bodies[1]).toMatchObject({
    plate_id: plates[0].id,
    models: ["box.stl"],
  });
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
  await expect(page.getByRole("alert")).toHaveCount(2);
  await expect(
    page.getByRole("button", { name: "設定へ（0）" }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "設定を再読込み" }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "プレート一覧へ" }),
  ).toHaveAttribute("href", "/");
});
