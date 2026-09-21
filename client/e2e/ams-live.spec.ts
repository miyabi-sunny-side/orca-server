import { test, expect } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";

test("compact AMS selection preserves editing during polling and changes start priority", async ({ page, request }) => {
  const c = JSON.parse(process.env.E2E_AMS_CONTEXT!);
  const output = process.env.E2E_EVIDENCE_DIR!;
  await mkdir(output, { recursive: true });
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto("/printers/p1/ams");
  const first = page.getByRole("listitem").filter({ has: page.getByRole("button", { name: "AMS 0 スロット 1の詳細", exact: true }) });
  await expect(first.getByRole("button", { name: /材料を選択/ })).toContainText("PLA 白");
  await expect(page.getByText(/設定温度:/)).not.toBeVisible();
  for (const theme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: theme });
    for (const width of [320, 375, 900]) {
      await page.setViewportSize({ width, height: 812 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: join(output, `ams-collapsed-${theme}-${width}.png`), fullPage: true });
    }
  }
  await first.getByRole("button", { name: "AMS 0 スロット 1の詳細", exact: true }).click();
  await expect(first.getByText(/設定温度:/)).toBeVisible();
  await first.getByLabel("印刷開始時の使用順").selectOption("0");
  await expect.poll(async () => (await (await request.get(c.resolve)).json()).preferred_slot.slot_index).toBe(0);
  await first.getByRole("button", { name: /材料を選択/ }).click();
  const search = first.getByRole("searchbox", { name: "材料を検索" });
  await search.fill("Fixture");
  await expect(first.getByRole("button", { name: /PLA 白.*Fixture/ })).toBeVisible();
  const refreshed = page.waitForResponse(r => r.url().endsWith("/api/printers/p1/ams") && r.request().method() === "GET");
  await refreshed;
  await expect(search).toHaveValue("Fixture"); await expect(search).toBeFocused();
  await expect(first.getByText(/設定温度:/)).toBeVisible();
  await search.press("ArrowDown"); await expect(first.getByRole("button", { name: "指定を解除", exact: true })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(first.getByRole("button", { name: /材料を選択/ })).toContainText("材料は未指定");
  await first.getByRole("button", { name: /材料を選択/ }).click();await search.fill("白");
  await first.getByRole("button", { name: /PLA 白.*Fixture/ }).click();
  await expect(first.getByRole("button", { name: /材料を選択/ })).toContainText("PLA 白");
  await first.getByRole("button", { name: /材料を選択/ }).click();await search.fill("Fixture");
  await request.post(c.control, { data: { print: { command: "push_status", msg: 1, ams: { ams: [{ id: "0", tray: [{ id: "0", tag_uid: "browser-replacement" }] }] } } } });
  await expect(first.getByRole("alert")).toContainText("観測情報が変わりました", { timeout: 12000 });
  await expect(search).toHaveValue("Fixture");await expect(first.getByRole("button", { name: /PLA 白.*Fixture/ })).toBeDisabled();
  await first.getByRole("button", { name: "選び直す", exact: true }).click();
  await first.getByRole("button", { name: /PLA 白.*Fixture/ }).click();
  await page.getByText("自動補充", { exact: true }).click();
  await expect(page.getByText("プリンターが非対応と報告しています。", { exact: true })).toBeVisible();
  await request.post(c.control, { data: { print: { command: "push_status", msg: 1, support_filament_backup: true, home_flag: 0 } } });
  await page.getByRole("button", { name: "状態を更新", exact: true }).click();
  await page.getByRole("button", { name: "有効にする", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("設定を送信しました");
  await expect(page.getByText("無効", { exact: true })).toBeVisible();
  await request.post(c.control, { data: { print: { command: "push_status", msg: 1, home_flag: 1024 } } });
  await expect(page.getByText("有効", { exact: true })).toBeVisible({ timeout: 12000 });
  await page.setViewportSize({ width: 375, height: 812 });
  await page.evaluate(() => { const sizes = Array.from(document.querySelectorAll<HTMLElement>("body,body *")).map(el => [el, parseFloat(getComputedStyle(el).fontSize)] as const); for (const [el, size] of sizes) el.style.fontSize = `${size * 2}px`; });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: join(output, "ams-expanded-text200.png"), fullPage: true });
});
