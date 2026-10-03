import { expect, test } from "@playwright/test";
import { queueFixture } from "./compact-fixture";

const reason =
  "The printer reported that the storage is full and the uploaded project could not be opened for printing ".repeat(
    3,
  ) + `/sdcard/${"x".repeat(120)}`;

for (const width of [320, 375, 900])
  for (const colorScheme of ["dark", "light"] as const) {
    test(`${width} ${colorScheme}: saved failure code and long reason stay readable with recovery`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 812 });
      await page.emulateMedia({ colorScheme });
      const { q } = await queueFixture(page);
      await page.goto("/");
      await page.locator(".current-job").waitFor();
      await expect(page.locator(".device-failure")).toHaveCount(0);
      Object.assign(q.current, {
        state: "needs_attention",
        last_error: "Printer rejected the start request",
        failure: {
          kind: "rejected",
          field: "err_code",
          code: "0500-4003",
          reason,
        },
      });
      Object.assign(q.printer.print, { state: "IDLE", error: 0x05004003 });
      Object.assign(q, {
        allowed: { next: false, retry: true, discard: true },
        recovery: { retry_reason: null, discard_reason: null },
      });
      await page.reload();
      const summary = page.locator(".current-job summary");
      await expect(summary).toContainText("要確認");
      await expect(summary).not.toContainText("0500-4003");
      const failure = page.locator(".device-failure");
      await expect(summary.locator("..")).not.toHaveAttribute("open");
      await expect(failure).toBeVisible();
      await expect(failure).toContainText("現在");
      await expect(failure).toContainText("0500-4003 意味は未確認");
      await expect(failure).not.toContainText(reason.trim());
      q.printer.print.error = 0;
      await page.reload();
      await expect(failure).toContainText("前回");
      await expect(failure).toContainText("0500-4003");
      await expect(failure).not.toContainText(reason.trim());
      await summary.click();
      const details = page.locator(".current-job .job-details");
      await expect(details).toContainText("コード: err_code 0500-4003");
      await expect(details).toContainText(`理由: ${reason.trim()}`);
      const box = (await details.boundingBox())!;
      expect(box.x + box.width).toBeLessThanOrEqual(width);
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      const retry = page.getByRole("button", {
        name: "再印刷",
        exact: true,
      });
      await retry.scrollIntoViewIfNeeded();
      await expect(retry).toBeEnabled();
      if (process.env.E2E_EVIDENCE_DIR)
        await page.screenshot({
          path: `${process.env.E2E_EVIDENCE_DIR}/failure-${width}-${colorScheme}.png`,
          fullPage: true,
        });
      Object.assign(q.current, {
        failure: { kind: "stopped", state: "FAILED" },
        last_error: "Print stopped; inspect the printer",
      });
      q.printer.print.error = 0;
      await page.reload();
      await expect(summary).toContainText("要確認");
      await expect(failure).toContainText("印刷停止");
    });
  }
