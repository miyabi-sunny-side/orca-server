import { expect, test } from "@playwright/test";
import type { QueueState } from "../src/lib/queue";

test("queue estimates update automatically and recover without sending a print", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  const { plate_id } = JSON.parse(process.env.E2E_ESTIMATE_CONTEXT!);
  const control = process.env.E2E_PRINTER_CONTROL!;
  const peer = async () => (await request.get(control)).json();
  const before = await peer();
  const row = () => page.locator(".plate-row").last();
  const add = async () => {
    await page.goto(`/plates/${plate_id}`);
    await page
      .getByRole("button", { name: "印刷キューへ", exact: true })
      .click();
    await page.getByRole("link", { name: "キューを見る", exact: true }).click();
  };
  const capture = async (name: string) => {
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(await page.locator(".btn.primary").count()).toBeLessThanOrEqual(1);
    await page.screenshot({
      path: `${process.env.E2E_EVIDENCE_DIR}/${name}.png`,
      fullPage: true,
    });
  };
  for (const colorScheme of ["dark", "light"] as const) {
    await page.setViewportSize({
      width: colorScheme === "dark" ? 375 : 900,
      height: 812,
    });
    await page.emulateMedia({ colorScheme });
    await request.post(control, { data: { slice_hold: true } });
    await add();
    await expect(row()).toContainText("試算中…");
    await expect(
      page.getByRole("button", { name: "空のプレートで印刷を開始" }),
    ).toBeEnabled();
    await expect(page.getByRole("checkbox")).toHaveCount(0);
    await capture(`estimate-calculating-${colorScheme}`);
    await request.post(control, { data: { slice_hold: false } });
    await expect(row()).toContainText("約19分");
    await capture(`estimate-ready-${colorScheme}`);
    await page.reload();
    await expect(row()).toContainText("約19分");
    await request.post(control, { data: { slice_fail: true } });
    await add();
    await expect(row()).toContainText("試算できませんでした");
    await expect(row()).not.toContainText("約0分");
    await capture(`estimate-failed-${colorScheme}`);
    await request.post(control, { data: { slice_fail: false } });
    await row()
      .getByRole("button", { name: /再試算/ })
      .click();
    await expect(row()).toContainText("約19分");
    const state: QueueState = await (
      await request.get("/api/queue?printer_id=p1")
    ).json();
    expect(state.waiting).toHaveLength(2);
    expect(state.waiting.every((j) => j.estimate?.seconds === 1140)).toBe(true);
    expect((await peer()).prints).toHaveLength(before.prints.length);
    expect((await peer()).uploads).toHaveLength(before.uploads.length);
    for (let i = 0; i < 2; i++) {
      await row().getByRole("button", { name: /削除/ }).click();
      await expect(page.locator(".plate-row")).toHaveCount(1 - i);
    }
  }
});
