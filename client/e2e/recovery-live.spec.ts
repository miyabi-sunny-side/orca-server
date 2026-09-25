import { expect, test } from "@playwright/test";
import type { QueueState } from "../src/lib/queue";

const ctx = JSON.parse(process.env.E2E_RECOVERY_CONTEXT || "{}");
const control = process.env.E2E_PRINTER_CONTROL!;

test("stopped jobs recover through editing, retry, removal and next", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  const state = async (): Promise<QueueState> =>
    (await request.get("/api/queue?printer_id=p1")).json();
  const count = async () => (await (await request.get(control)).json()).count;
  const report = async (name: string) => {
    await request.post(control, { data: { state: name } });
    await expect
      .poll(async () => (await state()).current?.state)
      .toBe(name === "RUNNING" ? "printing" : "needs_attention");
  };
  const capture = async (name: string) => {
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await expect(page.locator(".btn.primary:visible")).toHaveCount(1);
    await page.screenshot({
      path: `${process.env.E2E_EVIDENCE_DIR}/${name}.png`,
      fullPage: true,
    });
  };
  await page.emulateMedia({ colorScheme: "light" });
  await page.setViewportSize({ width: 900, height: 812 });
  await page.goto("/queue?printer_id=p1");
  await page
    .getByRole("button", { name: "空のプレートで印刷を開始", exact: true })
    .click();
  await expect.poll(count).toBe(1);
  await report("RUNNING");
  await report("FAILED");
  await page.locator(".current-job summary").click();
  await page
    .getByRole("link", { name: "プレートの条件を編集", exact: true })
    .click();
  await page.locator("summary", { hasText: "詳細設定" }).click();
  await page.getByLabel("充填率（%）").fill("25");
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await expect(page.getByRole("button", { name: "構成を編集" })).toBeVisible();
  await page.goto("/queue?printer_id=p1");
  const retry = page.getByRole("button", {
    name: "取り外した・最初から再印刷",
    exact: true,
  });
  await expect(retry).toBeEnabled();
  await capture("recovery-light");
  await retry.click();
  await expect.poll(count).toBe(2);
  await report("RUNNING");
  await report("FAILED");
  await page.emulateMedia({ colorScheme: "dark" });
  await page.setViewportSize({ width: 375, height: 812 });
  await capture("recovery-dark");
  await page
    .getByRole("button", {
      name: "取り外した・現在のジョブを除く",
      exact: true,
    })
    .click();
  await expect.poll(async () => (await state()).current).toBeNull();
  await page.reload();
  const next = page.getByRole("button", {
    name: "空のプレートで印刷を開始",
    exact: true,
  });
  await expect(next).toBeEnabled();
  expect((await state()).printer.ready_to_print).toBe(false);
  await capture("discard-next-dark");
  await next.click();
  await expect.poll(count).toBe(3);
  expect((await state()).current?.id).toBe(ctx.next_job);
  await report("RUNNING");
  await expect(page.locator(".current-job")).toContainText("印刷中");
  expect(await count()).toBe(3);
});
