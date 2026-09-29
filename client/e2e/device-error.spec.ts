import { expect, test } from "@playwright/test";
import { queueFixture } from "./compact-fixture";

for (const width of [320, 375, 900])
  for (const colorScheme of ["dark", "light"] as const)
    test(`${width} ${colorScheme}: paused printer explains the error without expanding the job`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 812 });
      await page.emulateMedia({ colorScheme });
      const { q, commands } = await queueFixture(page);
      Object.assign(q.current, {
        state: "needs_attention",
        last_error: "Printer reported an error; inspect the printer",
        failure: {
          kind: "device_error",
          field: "print_error",
          code: "0300-8010",
          state: "PAUSE",
        },
      });
      Object.assign(q.printer.print, { state: "PAUSE", error: 0x03008010 });
      Object.assign(q, {
        waiting: [],
        recovery: {
          retry_reason:
            "Wait for a matching terminal report for the previous start",
          discard_reason:
            "Wait for a matching terminal report for the previous start",
        },
      });
      await page.goto("/");
      const failure = page.locator(".device-failure");
      await expect(page.locator(".current-job")).not.toHaveAttribute("open");
      await expect(failure).toBeVisible();
      await expect(failure).toContainText("0300-8010");
      await expect(failure).toContainText("ホットエンド冷却ファン");
      await expect(failure).toContainText("PAUSE");
      await expect(failure).toBeInViewport();
      await expect(page.getByText(/前の開始結果が不明です/)).toHaveCount(0);
      const help = failure.getByRole("link", { name: /公式/ });
      const url = await help.getAttribute("href");
      expect(new URL(url!).hostname).toBe("e.bambulab.com");
      expect(new URL(url!).searchParams.get("e")).toBe("03008010");
      await page.context().route(url!, (route) =>
        route.fulfill({
          contentType: "text/html",
          body: "<title>Bambu Lab help</title>",
        }),
      );
      await help.focus();
      await expect(help).toBeFocused();
      const popup = page.waitForEvent("popup");
      await help.press("Enter");
      const opened = await popup;
      await opened.waitForLoadState();
      expect(opened.url()).toBe(url);
      await opened.close();
      await page.reload();
      await expect(failure).toContainText("0300-8010");
      await expect(page.locator(".current-job")).not.toHaveAttribute("open");
      await expect(
        page.getByRole("button", {
          name: "取り外した・最初から再印刷",
          exact: true,
        }),
      ).toBeDisabled();
      if (process.env.E2E_EVIDENCE_DIR)
        await page.screenshot({
          path: `${process.env.E2E_EVIDENCE_DIR}/device-error-${width}-${colorScheme}.png`,
          fullPage: true,
        });
      await page.addStyleTag({
        content:
          ":root { --fs-xs:24px; --fs-sm:28px; --fs-md:30px; --fs-lg:32px; --fs-xl:34px; }",
      });
      await help.scrollIntoViewIfNeeded();
      await expect(help).toBeInViewport();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      Object.assign(q.current, {
        state: "printing",
        failure: null,
        last_error: null,
      });
      Object.assign(q.printer.print, { state: "RUNNING", error: 0 });
      await expect(failure).toHaveCount(0);
      expect(commands).toHaveLength(0);
    });

test("unmanaged and unknown errors remain visible, while stale reports are labelled", async ({
  page,
}) => {
  const { q, commands } = await queueFixture(page);
  const job = q.current;
  Object.assign(q, { current: null, waiting: [] });
  Object.assign(q.printer.print, { state: "PAUSE", error: 0xffff1234 });
  await page.goto("/");
  const failure = page.locator(".device-failure");
  await expect(failure).toContainText("FFFF-1234");
  await expect(failure).toContainText("未確認");
  await expect(failure.getByRole("link", { name: /公式/ })).toBeVisible();
  Object.assign(job, {
    state: "needs_attention",
    failure: { kind: "stopped", state: "FAILED" },
  });
  Object.assign(q, { current: job });
  Object.assign(q.printer, { connection: "disconnected", synchronized: false });
  await expect(failure).toContainText("保存された印刷の報告");
  await expect(failure).toContainText("コード未取得");
  await expect(failure).not.toContainText("FFFF-1234");
  await expect(failure.getByRole("link")).toHaveCount(0);
  expect(commands).toHaveLength(0);
});
