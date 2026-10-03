import { test, expect, type Page } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";

const fits = (page: Page) =>
  page.evaluate(() => document.documentElement.scrollWidth <= innerWidth);

test("状態を更新 asks the printer, waits for its report and keeps the page state", async ({ page, request }) => {
  const c = JSON.parse(process.env.E2E_AMS_REFRESH_CONTEXT!);
  const output = process.env.E2E_EVIDENCE_DIR!;
  await mkdir(output, { recursive: true });
  const control = (data: object) => request.post(c.control, { data });
  const pushes = async () => (await (await request.get(c.control)).json()).pushes as number;
  const shoot = async (name: string) => {
    for (const theme of ["dark", "light"] as const) {
      await page.emulateMedia({ colorScheme: theme });
      for (const width of [320, 375, 900]) {
        await page.setViewportSize({ width, height: 812 });
        expect(await fits(page)).toBe(true);
        await page.screenshot({ path: join(output, `ams-refresh-${name}-${theme}-${width}.png`), fullPage: true });
      }
    }
    await page.setViewportSize({ width: 375, height: 812 });
  };

  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto("/printers");
  await page.getByRole("link", { name: "AMS", exact: true }).first().click();
  await expect(page).toHaveURL(/\/printers\/p1\/ams$/);
  const row = (n: number) =>
    page.getByRole("listitem").filter({ has: page.getByRole("button", { name: `AMS 0 スロット ${n}の詳細`, exact: true }) });
  const refresh = page.getByRole("button", { name: "状態を更新", exact: true });
  const fetching = page.getByRole("status").filter({ hasText: "本体から取得しています" });
  const done = page.getByRole("status").filter({ hasText: "プリンターの最新状態を反映しました" });
  await expect(row(2).getByRole("button", { name: /材料を選択/ })).toContainText("空");

  // Keep an open detail and a search in progress across the refresh.
  await row(1).getByRole("button", { name: "AMS 0 スロット 1の詳細", exact: true }).click();
  await row(1).getByRole("button", { name: /材料を選択/ }).click();
  const search = row(1).getByRole("searchbox", { name: "材料を検索" });
  await search.fill("Fixture");

  // An ordinary view read that started before the re-read finishes must not win.
  const old = await (await request.get("/api/printers/p1/ams")).json();
  let holding = true;
  let release!: () => void;
  const released = new Promise<void>((resolve) => (release = resolve));
  await page.route("**/api/printers/p1/ams", async (route) => {
    if (holding && route.request().method() === "GET") {
      await released;
      await route.fulfill({ json: old });
    } else await route.continue();
  });

  await control({ reply: null });
  const before = await pushes();
  const poll = page.waitForRequest((r) => r.url().endsWith("/api/printers/p1/ams") && r.method() === "GET", { timeout: 8000 });
  await refresh.click();
  await expect(fetching).toBeVisible();
  await expect(refresh).toBeDisabled();
  await expect.poll(pushes).toBe(before + 1);
  await poll;
  await shoot("fetching");
  await page.emulateMedia({ colorScheme: "light" });
  await control({ send: c.white });
  await expect(done).toBeVisible();
  await expect(refresh).toBeEnabled();
  const stale = page.waitForResponse((r) => r.url().endsWith("/api/printers/p1/ams") && r.request().method() === "GET");
  release();
  await stale;
  holding = false;
  await page.waitForTimeout(300);
  // Read once: a retrying assertion would let the next 5-second poll hide a stale view.
  expect(await row(2).getByRole("button", { name: /材料を選択/ }).textContent()).toContain("材料は未指定");
  await row(2).getByRole("button", { name: "AMS 0 スロット 2の詳細", exact: true }).click();
  await expect(row(2).getByText("装填あり", { exact: false })).toBeVisible();
  await expect(row(2).getByText("#FFFFFFFF")).toBeVisible();
  await expect(search).toHaveValue("Fixture");
  await expect(row(1).getByText(/設定温度:/)).toBeVisible();
  await shoot("done");

  // The same content again is a completed re-read; the keyboard reaches the button.
  await control({ reply: c.white });
  await refresh.focus();
  await page.keyboard.press("Enter");
  await expect(done).toBeVisible();
  expect(await pushes()).toBe(before + 2);

  // Reading in progress is not presented as a finished read.
  await control({ reply: c.reading });
  await refresh.click();
  await expect(page.getByRole("status").filter({ hasText: "AMSはまだ材料を読み取り中です" })).toBeVisible();

  // A lost connection is an alert with its cause; the same button retries.
  await control({ reply: null });
  await refresh.click();
  await expect.poll(pushes).toBe(before + 4);
  await control({ disconnect: true });
  const alert = page.getByRole("alert").filter({ hasText: "プリンターに接続できないため" });
  await expect(alert).toBeVisible();
  await expect(refresh).toBeEnabled();
  await shoot("failed");
  await control({ reply: c.white });
  await expect
    .poll(async () => (await (await request.get("/api/printer/status?printer_id=p1")).json()).synchronized, { timeout: 20000 })
    .toBe(true);
  await refresh.click();
  await expect(done).toBeVisible();
  await expect(alert).toHaveCount(0);

  await page.evaluate(() => {
    const sizes = Array.from(document.querySelectorAll<HTMLElement>("body,body *")).map((el) => [el, parseFloat(getComputedStyle(el).fontSize)] as const);
    for (const [el, size] of sizes) el.style.fontSize = `${size * 2}px`;
  });
  expect(await fits(page)).toBe(true);
  await page.screenshot({ path: join(output, "ams-refresh-text200.png"), fullPage: true });
});
