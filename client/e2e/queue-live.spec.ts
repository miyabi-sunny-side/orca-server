import { expect, test } from "@playwright/test";
import { writeFileSync } from "node:fs";
import type { Action, QueueState } from "../src/lib/queue";
import type { Plate } from "../src/lib/api";

test("mobile queue drives isolated P1 once per confirmed action", async ({ page, request }) => {
  test.setTimeout(180_000);
  page.setDefaultTimeout(12_000);
  const control = process.env.E2E_PRINTER_CONTROL!;
  const state = async (): Promise<QueueState> => (await request.get("/api/queue")).json();
  const peer = async () => (await request.get(control)).json();
  const command = async (action: Action) => {
    const q = await state();
    const response = await request.post("/api/queue", { data: { generation: q.generation, request_id: q.request_id, action } });
    expect(response.status()).toBe(200); return response.json();
  };
  const report = async (name: string) => {
    await request.post(control, { data: { state: name } });
    await expect.poll(async () => (await state()).current?.phase).toBe(name === "RUNNING" ? "printing" : "awaiting_removal");
  };
  const plates: Plate[] = (await (await request.get("/api/plates")).json()).sort((a: Plate, b: Plate) => a.name.localeCompare(b.name));
  let displayTheme = "dark";
  const add = async (index: number, slot: number) => {
    await page.goto(`/plates/${plates[index].id}`);
    await page.getByRole("link", { name: "印刷キューへ" }).click();
    await page.getByRole("combobox", { name: "使用するAMSスロット" }).selectOption(String(slot));
    if (index === 0) await capture(`add-${displayTheme}`);
    await page.getByRole("button", { name: "キューに追加", exact: true }).click();
    await expect(page).toHaveURL(/\/queue\?printer_id=p1$/);
    await expect(page.getByRole("status")).toHaveText("キューに追加しました");
  };
  const confirm = () => page.getByRole("checkbox", { name: "造形物を取り外し、空のビルドプレートを戻しました" });
  const next = () => page.getByRole("button", { name: "次を印刷", exact: true });
  const capture = async (name: string) => {
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(await page.locator(".btn.primary").count()).toBeLessThanOrEqual(1);
    await page.screenshot({ path: `${process.env.E2E_EVIDENCE_DIR}/${name}.png`, fullPage: true });
  };
  let replayVerified = false;
  for (const colorScheme of ["dark", "light"] as const) {
    displayTheme = colorScheme;
    await page.setViewportSize({ width: 375, height: 812 });
    await page.emulateMedia({ colorScheme });
    const before = (await peer()).prints.length;
    await add(0, 3); await add(1, 0);
    await page.getByRole("button", { name: `${plates[1].name}を前へ` }).click();
    await expect(page.getByRole("listitem").first()).toContainText(plates[1].name);
    await page.getByRole("button", { name: `${plates[1].name}を後へ` }).click();
    await expect(page.getByRole("listitem").first()).toContainText(plates[0].name);
    await page.getByRole("button", { name: `${plates[1].name}を削除` }).click();
    await expect(page.getByRole("listitem")).toHaveCount(1);
    await add(1, 0);
    await expect(next()).toBeDisabled();
    await confirm().focus(); await page.keyboard.press("Space");
    await expect(next()).toBeEnabled();
    await capture(`ready-375-${colorScheme}`);
    let releaseRead: (() => void) | undefined;
    let readWaiting = false;
    if (colorScheme === "dark") {
      const old = await state();
      const gate = new Promise<void>(resolve => { releaseRead = resolve; });
      await page.route("**/api/queue?*", async route => {
        if (route.request().method() === "GET") { readWaiting = true; await gate; await route.fulfill({ json: old }); }
        else await route.continue();
      });
      await expect.poll(() => readWaiting).toBe(true);
      await next().evaluate((button: HTMLButtonElement) => { button.click(); button.click(); });
    } else {
      const other = await state();
      const [response] = await Promise.all([
        request.post("/api/queue", { data: { generation: other.generation, request_id: other.request_id, action: { type: "next", expected_job: other.waiting[0].id, cleared: true } } }),
        next().evaluate((button: HTMLButtonElement) => { button.click(); button.click(); }),
      ]);
      expect([200, 409]).toContain(response.status());
    }
    await expect.poll(async () => (await peer()).prints.length).toBe(before + 1);
    if (releaseRead) {
      await expect(page.getByLabel("現在の印刷")).toContainText(plates[0].name);
      releaseRead(); await page.unrouteAll({ behavior: "wait" });
      await expect(page.getByLabel("現在の印刷")).toContainText(plates[0].name);
    } else if (await page.getByRole("alert").count()) {
      await page.getByRole("button", { name: "最新状態を読み直す" }).click();
    }
    expect((await peer()).prints.at(-1).ams_mapping).toEqual([3]);
    await report("RUNNING");
    await expect(page.getByLabel("現在の印刷")).toContainText("印刷中");
    await expect(next()).toHaveCount(0);
    await report("FINISH");
    await expect(page.getByLabel("現在の印刷")).toContainText("取り外し待ち");
    await expect(next()).toBeDisabled();
    expect((await peer()).prints.length).toBe(before + 1);
    await capture(`removal-375-${colorScheme}`);
    let lost = false, body = "";
    if (colorScheme === "dark") await page.route("**/api/queue?*", async route => {
      if (route.request().method() === "POST" && route.request().postDataJSON().action.type === "next") {
        if (!lost) { lost = true; body = route.request().postData()!; await route.fetch(); await route.abort("failed"); return; }
        expect(route.request().postData()).toBe(body); replayVerified = true;
      }
      await route.continue();
    });
    await confirm().check(); await next().click();
    if (colorScheme === "dark") {
      await expect(page.getByText("送信結果が不明です。別の印刷を始めず、同じ要求の結果を確認します。")).toBeVisible();
      await page.getByRole("button", { name: "同じ要求を再確認" }).click();
      await expect(page.getByRole("alert")).toHaveCount(0);
      await page.unroute("**/api/queue?*");
    }
    await expect.poll(async () => (await peer()).prints.length).toBe(before + 2);
    expect((await peer()).prints.at(-1).ams_mapping).toEqual([0]);
    await report("RUNNING"); await report("FINISH");
    await expect(page.getByRole("button", { name: "取り外しを完了" })).toBeVisible();
    await confirm().check(); await page.getByRole("button", { name: "取り外しを完了" }).click();
    await expect(page.getByLabel("現在の印刷")).toHaveCount(0);
  }
  expect(replayVerified).toBe(true);
  // A second HTTP client advances while this screen is stale. Its old click must not start the following job.
  await add(2, 3); await add(3, 0);
  await confirm().check(); await next().click();
  await expect.poll(async () => (await peer()).prints.length).toBe(5);
  await report("RUNNING"); await report("FINISH");
  await expect(page.getByLabel("現在の印刷")).toContainText("取り外し待ち");
  const stale = await state();
  await page.route("**/api/queue?*", route => route.request().method() === "GET" ? route.fulfill({ json: stale }) : route.continue());
  await confirm().check();
  await command({ type: "add", plate_id: plates[0].id, revision: plates[0].revision, ams_slot: 3 });
  await command({ type: "next", expected_job: stale.waiting[0].id, cleared: true });
  await expect.poll(async () => (await peer()).prints.length).toBe(6);
  await report("RUNNING"); await report("FINISH");
  await next().click();
  await expect(page.getByRole("alert")).toContainText("状態が変わりました");
  await page.unroute("**/api/queue?*");
  await page.getByRole("button", { name: "最新状態を読み直す" }).click();
  await expect(page.getByLabel("現在の印刷")).toContainText(plates[3].name);
  await expect(confirm()).not.toBeChecked();
  await expect(next()).toBeDisabled();
  expect((await peer()).prints.length).toBe(6);
  await page.getByRole("button", { name: `${plates[0].name}を削除` }).click();
  await confirm().check(); await page.getByRole("button", { name: "取り外しを完了" }).click();
  // A transfer failure remains visible until an explicit checked retry.
  await add(2, 3); await request.post(control, { data: { fail_upload: true } });
  await confirm().check(); await next().click();
  await expect(page.getByRole("alert")).toContainText("印刷データを転送できませんでした");
  const retry = page.getByRole("button", { name: "同じプレートを再印刷" });
  await expect(retry).toBeDisabled();
  expect((await peer()).prints.length).toBe(6);
  await page.setViewportSize({ width: 320, height: 812 });
  for (const colorScheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme }); await capture(`failed-320-${colorScheme}`);
  }
  await page.addStyleTag({ content: ":root { --fs-xs:24px; --fs-sm:28px; --fs-md:30px; --fs-lg:32px; --fs-xl:34px; }" });
  await capture("failed-320-text-200");
  await confirm().check(); await retry.click();
  await expect.poll(async () => (await peer()).prints.length).toBe(7);
  await report("RUNNING"); await report("FINISH");
  await expect(page.getByRole("button", { name: "取り外しを完了" })).toBeVisible();
  await confirm().check(); await page.getByRole("button", { name: "取り外しを完了" }).click();
  await expect(page.getByText("待機中のプレートはありません。プレートの詳細から追加できます。")).toBeVisible();
  await page.goto(`/queue?plate=${plates[0].id}`);
  await page.getByRole("combobox", { name: "使用するAMSスロット" }).selectOption("3");
  await request.post(control, { data: { revise_plate: plates[0].id } });
  await page.getByRole("button", { name: "キューに追加", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("状態が変わりました");
  await expect(page.getByRole("heading", { name: `${plates[0].name}（更新）` })).toBeVisible();
  expect((await state()).waiting).toHaveLength(0);
  await capture("revised-plate");
  await page.getByRole("button", { name: "キューに追加", exact: true }).click();
  await expect(page).toHaveURL(/\/queue\?printer_id=p1$/);
  await expect(page.getByRole("listitem")).toContainText(`${plates[0].name}（更新）`);
  await page.getByRole("button", { name: `${plates[0].name}（更新）を削除` }).click();
  writeFileSync(`${process.env.E2E_EVIDENCE_DIR}/result.json`, JSON.stringify({ realApi: true, isolatedMqttFtps: true, darkLightMobile: true, doubleClickOnce: true, concurrentClientOnce: true, lateReadIgnored: true, removalRequired: true, exactReplayAfterLostResponse: replayVerified, staleClientNoAdvance: true, orderAndRemove: true, failedUploadExplicitRetry: true, revisedPlateRequiresFreshClick: true, keyboardAnd200Percent: true, prints: (await peer()).prints.length }, null, 2));
});
