import { expect, test } from "@playwright/test";
import { writeFileSync } from "node:fs";
import type { Action, QueueState } from "../src/lib/queue";
import type { Plate } from "../src/lib/api";

test("queue starts and continues with one action on mobile and desktop", async ({ page, request }) => {
  test.setTimeout(180_000);
  page.setDefaultTimeout(12_000);
  page.on("dialog", () => { throw new Error("Unexpected extra confirmation"); });
  const control = process.env.E2E_PRINTER_CONTROL!;
  const state = async (): Promise<QueueState> => (await request.get("/api/queue")).json();
  const peer = async () => (await request.get(control)).json();
  const command = async (action: Action) => {
    const q = await state();
    const response = await request.post("/api/queue", { data: { epoch: q.epoch, generation: q.generation, request_id: q.request_id, action } });
    expect(response.status()).toBe(200); return response.json();
  };
  const report = async (name: string) => {
    await request.post(control, { data: { state: name } });
    await expect.poll(async () => (await state()).current?.state).toBe(name === "RUNNING" ? "printing" : "awaiting_removal");
  };
  await page.goto('/plates/new');
  await page.getByRole('checkbox').check();
  await page.getByRole('button', { name: '構成を確認（1）' }).click();
  await page.getByRole('textbox', { name: 'プレート名' }).fill('A · 机の配線整理・ケーブルホルダー');
  await page.getByRole('spinbutton').fill('2');
  await page.getByRole('button', { name: '保存', exact: true }).click();
  await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);
  await page.reload();
  await expect(page.getByRole('heading', { name: 'A · 机の配線整理・ケーブルホルダー' })).toBeVisible();
  const materials = await (await request.get('/api/filaments')).json();
  const slots = (await (await request.get('/api/printers/p1/ams')).json()).slots;
  const specification = (slot: number) => ({ ams_slot_id: slots.find((s: any) => s.slot_index === slot).id,
    filament_id: materials.find((f: any) => f.name === (slot === 3 ? 'PLA 青' : 'PLA 白')).id,
    required_machine_profile_key: 'Bambu Lab P1S 0.4 nozzle', process_profile_key: '0.20mm Standard @BBL X1C', bed_type: 'Textured PEI Plate' });
  const plates: Plate[] = (await (await request.get("/api/plates")).json()).sort((a: Plate, b: Plate) => a.name.localeCompare(b.name));
  let displayTheme = "dark";
  const configure = async(index:number,slot:number) => {
    const existing=await (await request.get(`/api/plates/${plates[index].id}`)).json();
    const {ams_slot_id,...conditions}=specification(slot);
    const response=await request.put(`/api/plates/${existing.id}`,{data:{name:existing.name,version:existing.version,models:existing.models,conditions}});
    expect(response.status()).toBe(200);plates[index]=await response.json();
  };
  const add = async (index: number, slot: number) => {
    await configure(index,slot);
    await page.goto(`/plates/${plates[index].id}`);
    if (index === 0) await capture(`add-${displayTheme}`);
    await page.getByRole("button", { name: "印刷キューへ", exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`/plates/${plates[index].id}$`));
    await expect(page.getByRole("status").filter({hasText:"キューに追加しました"})).toBeVisible();
    await page.getByRole('link',{name:'キューを見る',exact:true}).click();
    await expect(page).toHaveURL(/\/queue\?printer_id=p1$/);
  };
  const confirm = () => page.getByRole("checkbox", { name: "造形物を取り外し、空のビルドプレートを戻しました" });
  const next = () => page.getByRole("button", { name: /^(空のプレートで印刷を開始|取り外した・次を印刷)$/ });
  const capture = async (name: string) => {
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(await page.locator(".btn.primary").count()).toBeLessThanOrEqual(1);
    await page.evaluate(() => window.scrollTo(0, 0));
    await page.screenshot({ path: `${process.env.E2E_EVIDENCE_DIR}/${name}.png`, fullPage: true });
  };
  let replayVerified = false;
  for (const colorScheme of ["dark", "light"] as const) {
    displayTheme = colorScheme;
    await page.setViewportSize({ width: colorScheme === "dark" ? 375 : 900, height: 812 });
    await page.emulateMedia({ colorScheme });
    const before = (await peer()).prints.length;
    await add(0, 0);
    await page.getByRole('link', { name: 'プレートの条件を編集' }).click();
    await page.getByRole('combobox', { name: 'フィラメント',exact:true }).selectOption(specification(3).filament_id);
    await capture(`edit-${colorScheme}`);
    await page.getByRole('button', { name: '保存',exact:true }).click();
    await expect(page.getByRole('button',{name:'構成を編集'})).toBeVisible();
    await page.goto('/queue?printer_id=p1');
    await add(1, 0);
    await page.getByRole("button", { name: `${plates[1].name}を前へ` }).click();
    await expect(page.getByRole("listitem").first()).toContainText(plates[1].name);
    await page.getByRole("button", { name: `${plates[1].name}を後へ` }).click();
    await expect(page.getByRole("listitem").first()).toContainText(plates[0].name);
    await page.getByRole("button", { name: `${plates[1].name}を削除` }).click();
    await expect(page.getByRole("listitem")).toHaveCount(1);
    await add(1, 0);
    await expect(next()).toBeEnabled();
    await expect(confirm()).toHaveCount(0);
    await capture(`ready-${colorScheme}`);
    let releaseRead: (() => void) | undefined;
    let readWaiting = false;
    if (colorScheme === "dark") {
      const old = await state();
      const gate = new Promise<void>(resolve => { releaseRead = resolve; });
      // Keep routing installed while the held response completes and the app fetches AMS.
      await page.route("**/api/queue?*", async route => {
        if (route.request().method() === "GET" && !readWaiting) {
          readWaiting = true; await gate; await route.fulfill({ json: old });
        } else await route.continue();
      });
      await expect.poll(() => readWaiting).toBe(true);
      await next().evaluate((button: HTMLButtonElement) => { button.click(); button.click(); });
    } else {
      const other = await state();
      const [response] = await Promise.all([
        request.post("/api/queue", { data: { epoch: other.epoch, generation: other.generation, request_id: other.request_id, action: { type: "next", expected_job: other.waiting[0].id, removed_job: other.current?.id ?? null, cleared: true } } }),
        next().evaluate((button: HTMLButtonElement) => { button.click(); button.click(); }),
      ]);
      expect([200, 409]).toContain(response.status());
    }
    await expect.poll(async () => (await peer()).prints.length).toBe(before + 1);
    if (releaseRead) {
      await expect(page.getByLabel("現在の印刷")).toContainText(plates[0].name);
      const delayedResponse = page.waitForResponse(response => response.url().includes("/api/queue?") && response.request().method() === "GET");
      releaseRead(); await (await delayedResponse).finished();
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
    await expect(confirm()).toHaveCount(0);
    await expect(next()).toHaveText("取り外した・次を印刷");
    await expect(next()).toBeEnabled();
    expect((await peer()).prints.length).toBe(before + 1);
    await capture(`removal-${colorScheme}`);
    if (colorScheme === "dark") {
      const inventory = await (await request.get('/api/printers/p1/ams')).json();
      const slot = inventory.slots.find((s: any) => s.slot_index === 0);
      const path = `/api/printers/p1/ams/${slot.id}`;
      expect((await request.put(path, { data: { revision: slot.revision, filament_id: null } })).status()).toBe(204);
      await expect(next()).toBeDisabled();
      await expect(page.getByText('保留:', { exact: false })).toBeVisible();
      await expect(confirm()).toHaveCount(0);
      expect((await peer()).prints.length).toBe(before + 1);
      await capture('held-dark');
      const refreshed = await (await request.get('/api/printers/p1/ams')).json();
      expect((await request.put(path, { data: { revision: refreshed.slots.find((s: any) => s.id === slot.id).revision, filament_id: slot.filament_id } })).status()).toBe(204);
      await expect(next()).toBeEnabled();
    }
    let lost = false, body = "";
    if (colorScheme === "dark") await page.route("**/api/queue?*", async route => {
      if (!replayVerified && route.request().method() === "POST" && route.request().postDataJSON().action.type === "next") {
        if (!lost) { lost = true; body = route.request().postData()!; await route.fetch(); await route.abort("failed"); return; }
        expect(route.request().postData()).toBe(body); replayVerified = true;
      }
      await route.continue();
    });
    await next().click();
    if (colorScheme === "dark") {
      await expect(page.getByText("送信結果が不明です。別の印刷を始めず、同じ要求の結果を確認します。")).toBeVisible();
      await page.getByRole("button", { name: "同じ要求を再確認" }).click();
      await expect(page.getByRole("alert")).toHaveCount(0);
    }
    await expect.poll(async () => (await peer()).prints.length).toBe(before + 2);
    expect((await peer()).prints.at(-1).ams_mapping).toEqual([0]);
    await report("RUNNING"); await report("FINISH");
    await expect(page.getByRole("button", { name: "取り外した", exact: true })).toBeVisible();
    await capture(`complete-${colorScheme}`);
    await expect(confirm()).toHaveCount(0); await page.getByRole("button", { name: "取り外した", exact: true }).click();
    await expect(page.getByLabel("現在の印刷")).toHaveCount(0);
  }
  expect(replayVerified).toBe(true);
  // A second HTTP client advances while this screen is stale. Its old click must not start the following job.
  await add(2, 3); await add(3, 0);
  await next().click();
  await expect.poll(async () => (await peer()).prints.length).toBe(5);
  await report("RUNNING"); await report("FINISH");
  await expect(page.getByLabel("現在の印刷")).toContainText("取り外し待ち");
  const stale = await state();
  let staleReads = true;
  await page.route("**/api/queue?*", route => staleReads && route.request().method() === "GET" ? route.fulfill({ json: stale }) : route.continue());
  await expect(confirm()).toHaveCount(0);
  await configure(0,3);
  await command({ type: "add", plate_id: plates[0].id, plate_version: plates[0].version });
  await command({ type: "next", expected_job: stale.waiting[0].id, removed_job: stale.current?.id ?? null, cleared: true });
  await expect.poll(async () => (await peer()).prints.length).toBe(6);
  await report("RUNNING"); await report("FINISH");
  await next().click();
  await expect(page.getByRole("alert")).toContainText("状態が変わりました");
  staleReads = false;
  await page.getByRole("button", { name: "最新状態を読み直す" }).click();
  await expect(page.getByLabel("現在の印刷")).toContainText(plates[3].name);
  await expect(confirm()).toHaveCount(0);
  await expect(next()).toBeEnabled();
  expect((await peer()).prints.length).toBe(6);
  await page.getByRole("button", { name: `${plates[0].name}を削除` }).click();
  await expect(confirm()).toHaveCount(0); await page.getByRole("button", { name: "取り外した", exact: true }).click();
  // A transfer failure remains visible until an explicit checked retry.
  await add(2, 3); await request.post(control, { data: { fail_upload: true } });
  await next().click();
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
  await expect(page.getByRole("button", { name: "取り外した", exact: true })).toBeVisible();
  await expect(confirm()).toHaveCount(0);
  await page.getByRole("button", { name: "取り外した", exact: true }).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByText("待機中のプレートはありません。プレートの詳細から追加できます。")).toBeVisible();
  writeFileSync(`${process.env.E2E_EVIDENCE_DIR}/result.json`, JSON.stringify({ realApi: true, isolatedMqttFtps: true, darkMobileLightDesktop: true, noExtraConfirmation: true, oneActionContinueAndFinish: true, doubleClickOnce: true, concurrentClientOnce: true, lateReadIgnored: true, removalRequired: true, exactReplayAfterLostResponse: replayVerified, staleClientNoAdvance: true, orderAndRemove: true, failedUploadExplicitRetry: true, compositionSavedWithoutSlicing: true, queuedSettingsEditable: true, keyboardAnd200Percent: true, prints: (await peer()).prints.length }, null, 2));
});
