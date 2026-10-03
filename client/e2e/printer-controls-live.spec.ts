import { test, expect, type Page } from "@playwright/test";

const shot = (page: Page, name: string) =>
  page.screenshot({
    path: `${process.env.E2E_EVIDENCE_DIR}/${name}.png`,
    fullPage: true,
  });

test("start options, print controls and the printer page act on the printer", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  const context = JSON.parse(process.env.E2E_DEVICE_CONTEXT!);
  const control = process.env.E2E_PRINTER_CONTROL!;
  const peer = async () => (await request.get(control)).json();
  const report = async (data: object) =>
    expect((await request.post(control, { data })).ok()).toBe(true);
  await page.setViewportSize({ width: 375, height: 812 });

  // The plate's start options are one collapsed section of the editor.
  await page.goto(`/plates/${context.plate}?edit=1`);
  const start = page.locator("details", { hasText: "印刷開始" });
  await start.locator("summary").click();
  await expect(start.getByLabel("ベッドレベリング")).toBeChecked();
  await expect(start.getByLabel("振動補正")).not.toBeChecked();
  await start.getByLabel("タイムラプス").uncheck();
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await expect(page).not.toHaveURL(/edit=1/);

  await page.goto("/");
  await expect(page.getByText("ノズル 212/220℃ · ベッド 60/60℃")).toBeVisible();
  await page.getByRole("button", { name: "印刷", exact: true }).click();
  await expect.poll(async () => (await peer()).prints.length).toBe(1);
  await report({ state: "RUNNING" });
  const pause = page.getByRole("button", { name: "一時停止" });
  await expect(pause).toBeEnabled();
  await shot(page, "home-printing-375-light");
  await pause.click();
  await expect(
    page.getByRole("status").filter({ hasText: "本体が受け付けました" }),
  ).toBeVisible();
  await report({ state: "PAUSE" });
  await page.getByRole("button", { name: "再開" }).click();
  await report({ state: "RUNNING" });
  await page.getByRole("button", { name: "停止", exact: true }).click();
  await page.getByRole("button", { name: "停止を確定" }).click();
  await report({ state: "FAILED" });
  await expect(
    page.getByRole("button", { name: "再印刷", exact: true }),
  ).toBeEnabled();

  // The printer page keeps routine actions as icons and everything else collapsed.
  await page.getByRole("link", { name: "本体の操作" }).click();
  await expect(page).toHaveURL(/\/printers\/p1\/control$/);
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  const opened = await page.locator("details[open]").count();
  expect(opened).toBe(0);
  await page.getByRole("button", { name: "照明を消す" }).click();
  await expect(page.getByRole("status")).toContainText("本体が受け付けました");
  await page.getByRole("button", { name: "カメラ画像を更新" }).click();
  const image = page.getByRole("img", { name: "カメラ画像" });
  await expect(image).toBeVisible();
  await expect
    .poll(() => image.evaluate((i: HTMLImageElement) => i.naturalWidth))
    .toBe(320);
  await expect(
    page.getByRole("link", { name: /HMS 0300_0D00_0001_0004 の公式解説/ }),
  ).toHaveAttribute("href", /e=03000D0000010004/);
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await shot(page, `control-375-${scheme}`);
  }

  const files = page.locator("details", { hasText: "ファイル" });
  await files.locator("summary").click();
  await files.getByRole("button", { name: "timelapse/" }).click();
  await expect(
    files.getByRole("link", { name: "video_1.mp4をダウンロード" }),
  ).toHaveAttribute("href", /path=%2Ftimelapse%2Fvideo_1.mp4/);
  await files.getByRole("button", { name: "video_1.mp4を削除" }).click();
  await files.getByRole("button", { name: "video_1.mp4の削除を確定" }).click();
  await expect(files.getByText("ファイルはありません")).toBeVisible();

  for (const section of ["温度・ファン・速度", "移動", "給材", "オプション"]) {
    await page
      .locator("details", { hasText: section })
      .locator("summary")
      .click();
  }
  const feed = page.locator("details", { hasText: "給材" });
  await feed.getByLabel("材質").fill("PETG");
  await feed.getByRole("button", { name: "適用" }).click();
  await expect(page.getByRole("status").first()).toContainText(
    "本体が受け付けました",
  );
  await page.setViewportSize({ width: 320, height: 700 });
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(320);
  await shot(page, "control-320-dark-open");
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.emulateMedia({ colorScheme: "light" });
  await shot(page, "control-1280-light-open");
});
