import { test, expect, type Page } from "@playwright/test";
import { queueFixture } from "./compact-fixture";
async function fixture(page: Page) {
  const f = await queueFixture(page);
  f.q.current.name = "bin · 1モデル10個";
  f.q.waiting[0].name = "セパレータ";
  let reason: string | null = null,
    lose = false,
    serial = 0,
    last = "";
  await page.route("**/api/queue?*", async (route) => {
    const plate = new URL(route.request().url()).searchParams.get("plate_id");
    if (route.request().method() === "POST") {
      const c = route.request().postDataJSON();
      f.commands.push(c);
      if (JSON.stringify(c) !== last) {
        if (c.generation !== f.q.generation)
          return route.fulfill({
            status: 409,
            json: { error: "Queue changed; reload" },
          });
        if (c.action.type === "add") {
          const source = [f.q.current, ...f.q.waiting].find(
            (j) => j.plate_id === c.action.plate_id,
          )!;
          f.q.waiting.push({
            ...source,
            id: `copy-${++serial}`,
            state: "queued",
            attempt_id: null,
            artifact_path: null,
            estimate: { state: "pending", seconds: null, error: null },
          });
        } else if (c.action.type === "remove")
          f.q.waiting = f.q.waiting.filter((j) => j.id !== c.action.job_id);
        f.q.generation++;
        f.q.request_id = `request-${f.q.generation}`;
        last = JSON.stringify(c);
      }
      if (lose) {
        lose = false;
        return route.abort("failed");
      }
    }
    return route.fulfill({
      json: {
        ...f.q,
        admission: plate
          ? { plate_version: 7, allowed: !reason, reason }
          : null,
      },
    });
  });
  return {
    ...f,
    hold: (value: string | null) => {
      reason = value;
    },
    lose: () => {
      lose = true;
    },
  };
}
const menu = (page: Page) => page.getByRole("dialog");
const row = (page: Page, id: string) => page.locator(`#job-${id} > summary`);
const duplicate = (page: Page) =>
  menu(page).getByRole("button", { name: "キュー複製", exact: true });
async function open(page: Page, id: string) {
  await row(page, id).click({ button: "right" });
  await expect(menu(page)).toBeVisible();
}
for (const width of [375, 900])
  for (const colorScheme of ["light", "dark"] as const) {
    test(`${width} ${colorScheme}: duplicate current and waiting without expanding rows`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 812 });
      await page.emulateMedia({ colorScheme });
      const { q, commands } = await fixture(page);
      await page.goto("/");
      await expect(page.locator(".job-summary")).toHaveCount(9);
      await open(page, q.current.id);
      await expect(
        menu(page).getByRole("button", { name: "プレート編集", exact: true }),
      ).toBeEnabled();
      await expect(duplicate(page)).toBeEnabled();
      await expect(
        menu(page).getByRole("button", { name: "キュー削除", exact: true }),
      ).toBeDisabled();
      await expect(menu(page)).toContainText("印刷中");
      const widths = await menu(page)
        .locator(".queue-menu > button")
        .evaluateAll((nodes) =>
          nodes.map((n) => n.getBoundingClientRect().width),
        );
      expect(widths).toHaveLength(3);
      expect(new Set(widths).size).toBe(1);
      if (process.env.E2E_EVIDENCE_DIR)
        await page.screenshot({
          path: `${process.env.E2E_EVIDENCE_DIR}/menu-${width}-${colorScheme}.png`,
        });
      await page.keyboard.press("Escape");
      await expect(row(page, q.current.id)).toBeFocused();
      for (const id of [q.waiting[0].id, q.current.id]) {
        await open(page, id);
        await expect(duplicate(page)).toBeEnabled();
        await duplicate(page).evaluate((el: HTMLButtonElement) => {
          el.click();
          el.click();
        });
        await expect(menu(page)).toHaveCount(0);
        await expect(
          page.getByRole("status").filter({ hasText: "キューを複製しました" }),
        ).toBeVisible();
        await expect(row(page, id)).toBeFocused();
      }
      expect(commands).toHaveLength(2);
      expect(commands[0].action).toEqual({
        type: "add",
        plate_id: q.waiting[0].plate_id,
        plate_version: 7,
      });
      expect(commands[1].action).toEqual({
        type: "add",
        plate_id: q.current.plate_id,
        plate_version: 7,
      });
      await expect(page.locator(".waiting-job")).toHaveCount(10);
      await expect(page.locator(".job-details:visible")).toHaveCount(0);
      await expect(page).toHaveURL("/");
    });
  }
test("polling retains identity and reflects removal, deletion and admission", async ({
  page,
}) => {
  const { q, commands, hold } = await fixture(page);
  await page.goto("/");
  const source = q.waiting[0];
  await open(page, source.id);
  await expect(duplicate(page)).toBeEnabled();
  q.waiting.reverse();
  q.generation++;
  hold("No confirmed AMS slot contains the plate material");
  await expect(menu(page)).toContainText("AMS");
  await expect(duplicate(page)).toBeDisabled();
  hold("Queue holds at most 100 waiting jobs");
  await expect(menu(page)).toContainText("100件");
  (source as any).plate_deleted = true;
  await expect(
    menu(page).getByRole("button", { name: "プレート編集" }),
  ).toBeDisabled();
  await expect(
    menu(page).getByRole("button", { name: "キュー削除" }),
  ).toBeEnabled();
  delete (source as any).plate_deleted;
  hold(null);
  await expect(duplicate(page)).toBeEnabled();
  await duplicate(page).click();
  await expect(menu(page)).toHaveCount(0);
  expect(commands[0].action.plate_id).toBe(source.plate_id);
  await open(page, source.id);
  q.waiting = q.waiting.filter((j) => j.id !== source.id);
  q.generation++;
  await expect(menu(page)).toContainText("キューにありません");
  for (const name of ["プレート編集", "キュー複製", "キュー削除"])
    await expect(
      menu(page).getByRole("button", { name, exact: true }),
    ).toBeDisabled();
  expect(commands).toHaveLength(1);
  await page.keyboard.press("Escape");
  await expect(row(page, q.waiting.at(-1)!.id)).toBeFocused();
});
test("lost response reuses one command; subsequent explicit duplication adds again", async ({
  page,
}) => {
  const f = await fixture(page);
  await page.goto("/");
  await open(page, f.q.current.id);
  await expect(duplicate(page)).toBeEnabled();
  f.lose();
  await duplicate(page).click();
  const retry = menu(page).getByRole("button", { name: "同じ要求を再確認" });
  await expect(retry).toBeVisible();
  await expect(duplicate(page)).toBeDisabled();
  await retry.click();
  await expect(menu(page)).toHaveCount(0);
  expect(f.commands).toHaveLength(2);
  expect(f.commands[0]).toEqual(f.commands[1]);
  await expect(page.locator(".waiting-job")).toHaveCount(9);
  await open(page, f.q.current.id);
  await expect(duplicate(page)).toBeEnabled();
  await duplicate(page).click();
  await expect(page.locator(".waiting-job")).toHaveCount(10);
  expect(f.commands[2].request_id).not.toBe(f.commands[0].request_id);
});
test("keyboard delete focuses adjacent row and edit opens the selected plate", async ({
  page,
}) => {
  const { q, commands } = await fixture(page);
  await page.goto("/");
  const [first, second] = q.waiting;
  await row(page, first.id).focus();
  await page.keyboard.press("Shift+F10");
  await expect(menu(page)).toBeVisible();
  await menu(page)
    .getByRole("button", { name: "キュー削除", exact: true })
    .click();
  await expect(row(page, first.id)).toHaveCount(0);
  await expect(row(page, second.id)).toBeFocused();
  expect(commands[0].action).toEqual({ type: "remove", job_id: first.id });
  await page.keyboard.press("ContextMenu");
  await expect(menu(page)).toBeVisible();
  await menu(page)
    .getByRole("button", { name: "プレート編集", exact: true })
    .click();
  await expect(page).toHaveURL(`/plates/${second.plate_id}?edit=1`);
});
test("long press survives finger-up; preparing/removal/attention states cannot be deleted", async ({
  browser,
}) => {
  const context = await browser.newContext({
    viewport: { width: 375, height: 812 },
    hasTouch: true,
    isMobile: true,
  });
  const page = await context.newPage();
  const { q, commands } = await fixture(page);
  await page.goto("/");
  await expect(row(page, q.current.id)).toBeVisible();
  const rect = (await row(page, q.current.id).boundingBox())!;
  const cdp = await context.newCDPSession(page);
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ x: rect.x + 140, y: rect.y + 25 }],
  });
  await expect(menu(page)).toBeVisible();
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchEnd",
    touchPoints: [],
  });
  await expect(duplicate(page)).toBeEnabled();
  await page.keyboard.press("Escape");
  await expect(page.locator(".job-details:visible")).toHaveCount(0);
  for (const state of ["preparing", "awaiting_removal", "needs_attention"]) {
    q.current.state = state;
    await open(page, q.current.id);
    await expect(duplicate(page)).toBeEnabled();
    await expect(
      menu(page).getByRole("button", { name: "キュー削除", exact: true }),
    ).toBeDisabled();
    await page.keyboard.press("Escape");
  }
  expect(commands).toHaveLength(0);
  await context.close();
});

test("late admission for the previous menu cannot authorize or disable another job", async ({
  page,
}) => {
  const { q, commands } = await fixture(page);
  let held = false;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const [first, second] = q.waiting;
  await page.route("**/api/queue?*", async (route) => {
    if (
      route.request().method() === "GET" &&
      new URL(route.request().url()).searchParams.get("plate_id") ===
        first.plate_id &&
      !held
    ) {
      held = true;
      await gate;
      await route.fulfill({
        json: {
          ...q,
          admission: {
            plate_version: 2,
            allowed: false,
            reason: "old admission",
          },
        },
      });
    } else await route.fallback();
  });
  await page.goto("/");
  await open(page, first.id);
  await expect.poll(() => held).toBe(true);
  await page.keyboard.press("Escape");
  await open(page, second.id);
  await expect(duplicate(page)).toBeDisabled();
  release();
  await expect(duplicate(page)).toBeEnabled();
  await expect(menu(page)).not.toContainText("old admission");
  await duplicate(page).click();
  await expect(menu(page)).toHaveCount(0);
  expect(commands[0].action).toEqual({
    type: "add",
    plate_id: second.plate_id,
    plate_version: 7,
  });
});

test("a selected waiting job that starts while the menu is open cannot be removed", async ({
  page,
}) => {
  const { q, commands } = await fixture(page);
  await page.goto("/");
  const source = q.waiting[0];
  await open(page, source.id);
  await expect(
    menu(page).getByRole("button", { name: "キュー削除", exact: true }),
  ).toBeEnabled();
  q.current = { ...source, state: "printing" };
  q.waiting = q.waiting.filter((j) => j.id !== source.id);
  q.generation++;
  await expect(
    menu(page).getByRole("button", { name: "キュー削除", exact: true }),
  ).toBeDisabled();
  await expect(menu(page)).toContainText("印刷中");
  await expect(duplicate(page)).toBeEnabled();
  expect(commands).toHaveLength(0);
});
