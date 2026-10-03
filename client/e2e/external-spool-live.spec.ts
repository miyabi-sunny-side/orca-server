import { test, expect } from "@playwright/test";

test("after unplugging the AMS the queue prints from the external spool without any feed choice", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  const control = process.env.E2E_PRINTER_CONTROL!;
  const peer = async () => (await request.get(control)).json();
  const report = async (data: object) =>
    expect((await request.post(control, { data })).ok()).toBe(true);
  const shot = async (name: string) => {
    for (const [width, scheme] of [
      [1280, "light"],
      [375, "dark"],
    ] as const) {
      await page.setViewportSize({ width, height: 812 });
      await page.emulateMedia({ colorScheme: scheme });
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth),
      ).toBeLessThanOrEqual(width);
      await page.screenshot({
        path: `${process.env.E2E_EVIDENCE_DIR}/${name}-${width}-${scheme}.png`,
        fullPage: true,
      });
    }
  };

  await page.goto("/");
  await expect(page.locator(".current-job summary")).toContainText("要確認");
  await expect(page.getByText(/AMS slot|AMSを確認/)).toHaveCount(0);
  await expect(page.getByLabel("給材")).toHaveCount(0);
  const waiting = page.locator("li details").first();
  await waiting.locator("summary").click();
  await expect(waiting.getByText("外部スプール ·")).toBeVisible();
  await shot("unplugged-queue");
  await page.getByRole("button", { name: "再印刷", exact: true }).click();
  await expect.poll(async () => (await peer()).prints.length).toBe(2);
  expect((await peer()).prints[1].use_ams).toBe(false);

  await report({ state: "RUNNING" });
  await expect(page.getByText("印刷中").first()).toBeVisible();
  await report({ state: "FINISH" });
  const next = page.getByRole("button", { name: "次を印刷", exact: true });
  await expect(next).toBeEnabled();
  await next.click();
  await expect.poll(async () => (await peer()).prints.length).toBe(3);
  expect((await peer()).prints[2].use_ams).toBe(false);

  await page.goto("/printers/p1/ams");
  await expect(
    page.getByText("AMS未接続 · 外部スプールで印刷します"),
  ).toBeVisible();
  await shot("unplugged-ams");
});
