import { test, expect, type Page } from "@playwright/test";
import { queueFixture } from "./compact-fixture";

async function fixture(page: Page) {
  const f = await queueFixture(page);
  const plate = {
    id: f.q.current.plate_id,
    name: "現在の名前",
    version: 7,
    conditions: { required_machine_profile_key: "Bambu Lab P1S 0.4 nozzle" },
    models: [],
  };
  const items = Array.from({ length: 50 }, (_, i) => ({
    id: 100 - i,
    name:
      i === 0
        ? "長いプレート名・前面の取り付け部品と追加のケース"
        : `部品 ${i}`,
    plate_id: plate.id,
    printer_id: "p1",
    completed_at: 1750000000 - i,
    available: i !== 2,
  }));
  let fail = false,
    empty = false,
    hold = false;
  await page.route("**/api/history*", (route) =>
    fail
      ? route.fulfill({
          status: 503,
          json: { error: "一時的に取得できません" },
        })
      : route.fulfill({
          json: {
            items: empty
              ? []
              : new URL(route.request().url()).searchParams.has("before")
                ? [{ ...items[0], id: 1, name: "以前の印刷" }]
                : items,
            next_cursor:
              empty || new URL(route.request().url()).searchParams.has("before")
                ? null
                : "1749999951:51",
          },
        }),
  );
  await page.route("**/api/plates/*", (route) =>
    route.fulfill({ json: plate }),
  );
  await page.route("**/api/queue?*", (route) => {
    if (route.request().method() === "POST") {
      f.commands.push(route.request().postDataJSON());
      f.q.generation++;
    }
    return route.fulfill({
      json: {
        ...f.q,
        request_id: `request-${f.q.generation}`,
        admission: {
          allowed: !hold,
          plate_version: 7,
          reason: hold
            ? "No confirmed AMS slot contains the plate material"
            : null,
        },
      },
    });
  });
  return {
    ...f,
    items,
    fail: (v: boolean) => (fail = v),
    empty: () => (empty = true),
    hold: (v: boolean) => (hold = v),
  };
}

test.use({ timezoneId: "Asia/Tokyo" });
for (const width of [320, 375, 900])
  for (const colorScheme of ["light", "dark"] as const) {
    test(`${width} ${colorScheme}: history menu, dates, paging and focus`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 812 });
      await page.emulateMedia({ colorScheme });
      const f = await fixture(page);
      await page.goto("/");
      await page
        .getByRole("link", { name: "プリント履歴", exact: true })
        .click();
      await expect(page).toHaveURL(/\/history$/);
      const rows = page.locator(".history-row");
      await expect(rows).toHaveCount(50);
      await expect(rows.first()).toContainText("2025/06/16 00:06:40");
      await expect(rows.first().locator("time")).toHaveAttribute(
        "datetime",
        "2025-06-15T15:06:40.000Z",
      );
      await rows.first().click({ button: "right" });
      const dialog = page.getByRole("dialog");
      await expect(dialog).toBeVisible();
      await expect(
        dialog.getByRole("button", { name: "キューに追加", exact: true }),
      ).toBeEnabled();
      await page.keyboard.press("Escape");
      await expect(rows.first()).toBeFocused();
      await rows.first().press("Shift+F10");
      await expect(dialog).toBeVisible();
      await dialog
        .getByRole("button", { name: "キューに追加", exact: true })
        .click();
      await expect(dialog).toHaveCount(0);
      await expect(rows.first()).toBeFocused();
      expect(f.commands).toHaveLength(1);
      expect(f.commands[0].action).toEqual({
        type: "add",
        plate_id: f.items[0].plate_id,
        plate_version: 7,
      });
      await expect(
        page.getByRole("status").filter({ hasText: "キューに追加しました" }),
      ).toBeVisible();
      await expect(
        page.getByRole("link", { name: "キューを見る" }),
      ).toHaveAttribute("href", "/queue?printer_id=p1");
      await rows.nth(2).press("ContextMenu");
      await expect(
        dialog.getByRole("button", { name: "キューに追加", exact: true }),
      ).toBeDisabled();
      await expect(dialog).toContainText(
        "プレートが削除されているため追加できません",
      );
      await page.locator(".scrim").click({ position: { x: 3, y: 3 } });
      await expect(rows.nth(2)).toBeFocused();
      if (process.env.E2E_EVIDENCE_DIR)
        await page.screenshot({
          path: `${process.env.E2E_EVIDENCE_DIR}/history-${width}-${colorScheme}.png`,
        });
      await page.getByRole("button", { name: "以前の履歴を読み込む" }).click();
      await expect(rows).toHaveCount(51);
      await expect(rows.last()).toContainText("以前の印刷");
      await page.addStyleTag({
        content:
          ":root { --fs-xs:24px; --fs-sm:28px; --fs-md:30px; --fs-lg:32px; --fs-xl:34px; }",
      });
      await rows.first().scrollIntoViewIfNeeded();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      await rows.first().click({ button: "right" });
      await expect(
        dialog.getByRole("button", { name: "キューに追加", exact: true }),
      ).toBeEnabled();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      if (process.env.E2E_EVIDENCE_DIR)
        await page.screenshot({
          path: `${process.env.E2E_EVIDENCE_DIR}/history-${width}-${colorScheme}-200.png`,
        });
    });
  }

test("load errors retry without discarding earlier pages; empty and unavailable conditions", async ({
  page,
}) => {
  const f = await fixture(page);
  f.fail(true);
  await page.goto("/history");
  await expect(page.getByRole("alert")).toContainText(
    "履歴を取得できませんでした",
  );
  f.fail(false);
  await page.getByRole("button", { name: "再試行" }).click();
  const rows = page.locator(".history-row");
  await expect(rows).toHaveCount(50);
  f.fail(true);
  await page.getByRole("button", { name: "以前の履歴を読み込む" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(rows).toHaveCount(50);
  f.fail(false);
  await page.getByRole("button", { name: "再試行" }).click();
  await expect(rows).toHaveCount(51);
  f.hold(true);
  await rows.first().click({ button: "right" });
  await expect(
    page
      .getByRole("dialog")
      .getByRole("button", { name: "キューに追加", exact: true }),
  ).toBeDisabled();
  await expect(page.getByRole("link", { name: "AMSを確認" })).toBeVisible();
  await expect(page.getByRole("link", { name: "プレート編集" })).toBeVisible();
  await page.keyboard.press("Escape");
  f.empty();
  await page.reload();
  await expect(page.getByText("プリント履歴はまだありません")).toBeVisible();
});

test("long press opens the shared menu while vertical touch movement scrolls", async ({
  browser,
}) => {
  const context = await browser.newContext({
    viewport: { width: 375, height: 520 },
    hasTouch: true,
    isMobile: true,
  });
  const page = await context.newPage();
  await fixture(page);
  await page.goto("/history");
  const rows = page.locator(".history-row");
  await expect(rows).toHaveCount(50);
  const cdp = await context.newCDPSession(page);
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x: 180, y: 450 }],
  });
  for (let y = 420; y >= 130; y -= 30)
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x: 180, y }],
    });
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchEnd",
    touchPoints: [],
  });
  await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(60);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.evaluate(() => scrollTo(0, 0));
  const box = (await rows.first().boundingBox())!;
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x: box.x + 100, y: box.y + 20 }],
  });
  await expect(page.getByRole("dialog")).toBeVisible();
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchEnd",
    touchPoints: [],
  });
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(rows.first()).toBeFocused();
  await context.close();
});

test("original printer wins over saved selection; unavailable references remain readable", async ({
  page,
}) => {
  const f = await fixture(page);
  const machine = "Bambu Lab P1S 0.4 nozzle";
  let original = true;
  await page.route("**/api/printers", (route) =>
    route.fulfill({
      json: [
        ...(original
          ? [{ id: "p1", name: "Original", machine_profile_key: machine }]
          : []),
        { id: "p2", name: "Alternative", machine_profile_key: machine },
      ],
    }),
  );
  await page.addInitScript(
    (id) => localStorage.setItem(`orca-plate-printer:${id}`, "p2"),
    f.items[0].plate_id,
  );
  await page.goto("/history");
  const row = page.locator(".history-row").first();
  await row.click({ button: "right" });
  await expect(page.getByLabel("追加先のプリンター")).toHaveValue("p1");
  await page.keyboard.press("Escape");
  original = false;
  await row.click({ button: "right" });
  await expect(page.getByRole("dialog")).toContainText("追加先: Alternative");
  await expect(
    page
      .getByRole("dialog")
      .getByRole("button", { name: "キューに追加", exact: true }),
  ).toBeEnabled();
  await page.keyboard.press("Escape");
  await page.route("**/api/plates/*", (route) =>
    route.fulfill({ status: 404, json: { error: "deleted" } }),
  );
  await row.click({ button: "right" });
  await expect(page.getByRole("dialog")).toContainText(
    "プレートが削除されているため追加できません",
  );
  await expect(
    page
      .getByRole("dialog")
      .getByRole("button", { name: "キューに追加", exact: true }),
  ).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(row).toContainText(f.items[0].name);
});

test("adding from deep history preserves the row position and shows the result", async ({
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await fixture(page);
  await page.goto("/history");
  const row = page.locator(".history-row").nth(30);
  await row.scrollIntoViewIfNeeded();
  const before = (await row.boundingBox())!.y;
  await row.click({ button: "right" });
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "キューに追加", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    page.getByRole("status").filter({ hasText: "キューに追加しました" }),
  ).toBeInViewport();
  expect(Math.abs((await row.boundingBox())!.y - before)).toBeLessThan(2);
  await expect(row).toBeFocused();
});
