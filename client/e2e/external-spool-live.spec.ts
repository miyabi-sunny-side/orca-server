import { test, expect } from "@playwright/test";

test("without an AMS the external spool is chosen, starts without the AMS and shows the failure", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  const context = JSON.parse(process.env.E2E_EXTERNAL_CONTEXT!);
  const control = process.env.E2E_PRINTER_CONTROL!;
  const peer = async () => (await request.get(control)).json();
  const report = async (data: object) =>
    expect((await request.post(control, { data })).ok()).toBe(true);
  await page.setViewportSize({ width: 375, height: 812 });

  await page.goto(`/plates/${context.plate}`);
  const feed = page.getByLabel("給材");
  await expect(feed).toHaveValue("external");
  await expect(
    page.getByRole("button", { name: "印刷キューへ" }),
  ).toBeEnabled();
  await page.screenshot({
    path: `${process.env.E2E_EVIDENCE_DIR}/external-add-light.png`,
    fullPage: true,
  });
  await page.setViewportSize({ width: 320, height: 700 });
  await page.emulateMedia({ colorScheme: "dark" });
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(320);
  await feed.screenshot({
    path: `${process.env.E2E_EVIDENCE_DIR}/external-feed-320-dark.png`,
  });
  await page.setViewportSize({ width: 375, height: 812 });
  await page.emulateMedia({ colorScheme: "light" });
  await page.getByRole("button", { name: "印刷キューへ" }).click();
  await expect(page.getByText("キューに追加しました")).toBeVisible();

  await page.goto("/");
  const job = page.locator("li details").first();
  await job.locator("summary").click();
  await expect(job.getByText("外部スプール ·")).toBeVisible();
  await expect(job.getByRole("link", { name: "AMSを確認" })).toHaveCount(0);
  await job.locator("summary").click({ button: "right" });
  const menu = page.getByRole("dialog");
  await expect(menu.getByRole("button", { name: "AMSで印刷" })).toBeEnabled();
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "印刷", exact: true }).click();
  await expect.poll(async () => (await peer()).prints.length).toBe(1);
  expect((await peer()).prints[0].use_ams).toBe(false);

  await report({ state: "RUNNING" });
  await expect(page.getByText("印刷中").first()).toBeVisible();
  await report({ state: "FAILED", print_error: 0x0300_8010 });
  await expect(page.getByText("0300-8010").first()).toBeVisible();
  await expect(
    page.getByText("ホットエンド冷却ファンの回転異常").first(),
  ).toBeVisible();
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({
      path: `${process.env.E2E_EVIDENCE_DIR}/external-failure-${scheme}.png`,
      fullPage: true,
    });
  }
});
