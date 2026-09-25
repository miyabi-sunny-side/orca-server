import { expect, test } from "@playwright/test";

test("saved plate calculation, failure recovery and normal print share one result", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  const { plate_id } = JSON.parse(process.env.E2E_ESTIMATE_CONTEXT!);
  const control = process.env.E2E_PRINTER_CONTROL!;
  const peer = async () => (await request.get(control)).json();
  const before = await peer();
  const status = () => page.getByLabel("プレートの試算");
  const capture = async (name: string) => {
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `${process.env.E2E_EVIDENCE_DIR}/${name}.png`,
      fullPage: true,
    });
  };
  const save = async (density: number) => {
    await page.getByRole("button", { name: "構成を編集", exact: true }).click();
    await page.getByText("詳細設定", { exact: true }).click();
    await page.getByLabel("充填率（%）").fill(String(density));
    await page.getByRole("button", { name: "保存", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "構成を編集", exact: true }),
    ).toBeVisible();
  };
  for (const [index, colorScheme] of (["dark", "light"] as const).entries()) {
    await page.setViewportSize({ width: index === 0 ? 375 : 320, height: 812 });
    await page.emulateMedia({ colorScheme });
    await page.goto(`/plates/${plate_id}`);
    await request.post(control, { data: { slice_hold: true } });
    await save(20 + index);
    await expect(status()).toContainText("試算中…");
    await capture(`saved-calculating-${colorScheme}`);
    expect((await peer()).prints).toHaveLength(before.prints.length);
    expect((await peer()).uploads).toHaveLength(before.uploads.length);
    await request.post(control, { data: { slice_hold: false } });
    await expect(status()).toContainText("約19分");
    await capture(`saved-ready-${colorScheme}`);
    await page.reload();
    await expect(status()).toContainText("約19分");
    await request.post(control, { data: { slice_fail: true } });
    await save(30 + index);
    await expect(status()).toContainText("試算できませんでした");
    await status().locator("summary").click();
    await expect(
      status().getByRole("button", { name: "プレートの条件を編集" }),
    ).toBeVisible();
    await capture(`saved-failed-${colorScheme}`);
    await request.post(control, { data: { slice_fail: false } });
    await status().getByRole("button", { name: "再試算", exact: true }).click();
    await expect(status()).toContainText("約19分");
    await page
      .getByRole("button", { name: "印刷キューへ", exact: true })
      .click();
    await page.getByRole("link", { name: "キューを見る", exact: true }).click();
    await expect(page.locator(".waiting-job").last()).toContainText("約19分");
    await capture(`shared-queue-${colorScheme}`);
    expect((await peer()).prints).toHaveLength(before.prints.length);
  }
  await page
    .getByRole("button", { name: "空のプレートで印刷を開始", exact: true })
    .click();
  await expect
    .poll(async () => (await peer()).prints.length)
    .toBe(before.prints.length + 1);
  await request.post(control, { data: { state: "RUNNING" } });
  await expect
    .poll(
      async () =>
        (await (await request.get("/api/queue?printer_id=p1")).json()).current
          ?.state,
    )
    .toBe("printing");
  await request.post(control, { data: { state: "FINISH" } });
  await expect
    .poll(
      async () =>
        (await (await request.get("/api/queue?printer_id=p1")).json()).current
          ?.state,
    )
    .toBe("awaiting_removal");
  expect((await peer()).prints).toHaveLength(before.prints.length + 1);
});
