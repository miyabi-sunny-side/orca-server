import { expect, test } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";

test("real model selection, slicing, saved search/reload and explicit reimport", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);
  page.setDefaultTimeout(10_000);
  await page.setViewportSize({ width: 375, height: 812 });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await expect(page.getByText("保存済みプレートはありません")).toBeVisible();
  await page.getByRole("link", { name: "新規作成" }).click();
  await expect(page.getByRole("checkbox")).toHaveCount(2);
  await page.getByRole("searchbox").fill("bx");
  await expect(page.getByRole("checkbox")).toHaveCount(1);
  await page.getByRole("searchbox").press("ArrowDown");
  await page.keyboard.press("Space");
  await page.getByRole("searchbox").fill("hldr");
  await expect(page.getByRole("checkbox")).toHaveCount(1);
  await page.getByRole("checkbox").check();
  await captureViews("selection");
  await page.getByRole("button", { name: "設定へ（2）" }).click();
  await page
    .getByRole("textbox", { name: "プレート名" })
    .fill("机の小物入れ・配線用パーツ");
  await page
    .getByRole("combobox", { name: "工程", exact: true })
    .selectOption("0.16mm Optimal @BBL X1C");
  await page
    .getByRole("combobox", { name: "材料", exact: true })
    .selectOption("Bambu PLA Basic @BBL X1C");
  await page
    .getByRole("combobox", { name: "プレート種類", exact: true })
    .selectOption("High Temp Plate");
  await captureViews("settings");
  await page.getByRole("button", { name: "配置して保存" }).click();
  await expect(page.getByRole("status")).toContainText(
    /取り込んでいます|配置・スライス中/,
  );
  await expect(
    page.getByRole("button", { name: /配置して保存|もう一度配置する/ }),
  ).toBeDisabled();
  await expect(page.getByRole("img", { name: /モデルの配置/ })).toBeVisible({
    timeout: 60_000,
  });
  const url = page.url();
  const id = new URL(url).pathname.split("/").pop();
  const saved = await (await request.get(`/api/plates/${id}`)).json();
  expect(saved.models).toHaveLength(2);
  expect(saved.settings.slicer.process).toBe("0.16mm Optimal @BBL X1C");
  const oldBytes = await (
    await request.get(`/api/plates/${id}/files/${saved.models[0].path}`)
  ).body();
  await expect(page.getByText("20.0 × 20.0 × 20.0 mm")).toHaveCount(2);
  async function captureViews(name: string) {
    for (const width of [320, 375, 900])
      for (const colorScheme of ["dark", "light"] as const) {
        await page.setViewportSize({ width, height: 812 });
        await page.emulateMedia({ colorScheme });
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true);
        await page.screenshot({
          path: `${process.env.E2E_EVIDENCE_DIR}/${name}-${width}-${colorScheme}.png`,
          fullPage: true,
        });
      }
  }
  await captureViews("detail");
  const download = page.waitForEvent("download");
  await page.getByRole("link", { name: "印刷データを取得" }).click();
  const file = await download;
  expect(await file.failure()).toBeNull();
  await file.saveAs(`${process.env.E2E_EVIDENCE_DIR}/browser-print.gcode.3mf`);
  await page.getByRole("link", { name: "プレート一覧へ" }).click();
  await page.getByRole("searchbox").fill("bxs");
  await expect(page.locator(".plate-row")).toHaveCount(1);
  await page.getByRole("searchbox").press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(url);
  await page.reload();
  await expect(page.getByRole("img", { name: /モデルの配置/ })).toBeVisible();
  // Change only our isolated scad fixture: browsing never overwrites a saved snapshot.
  const source = readFileSync(process.env.E2E_MODEL_FILE!);
  for (let triangle = 0; triangle < source.readUInt32LE(80); triangle++) {
    for (
      let offset = 96 + triangle * 50;
      offset < 132 + triangle * 50;
      offset += 4
    )
      source.writeFloatLE(source.readFloatLE(offset) * 1.5, offset);
  }
  writeFileSync(process.env.E2E_MODEL_FILE!, source);
  await page.reload();
  await expect(page.getByText("20.0 × 20.0 × 20.0 mm")).toHaveCount(2);
  expect(
    await (
      await request.get(`/api/plates/${id}/files/${saved.models[0].path}`)
    ).body(),
  ).toEqual(oldBytes);
  await page.getByText("元モデルを更新", { exact: true }).click();
  await page.getByRole("button", { name: "取り込み直して配置" }).click();
  await expect(page.getByText("30.0 × 30.0 × 30.0 mm")).toBeVisible({
    timeout: 60_000,
  });
  await expect(page.getByText("20.0 × 20.0 × 20.0 mm")).toHaveCount(1);
  const updated = await (await request.get(`/api/plates/${id}`)).json();
  expect(updated.id).toBe(saved.id);
  expect(updated.revision).not.toBe(saved.revision);
  expect(updated.settings).toEqual(saved.settings);
  expect(await (await request.get("/api/plates")).json()).toHaveLength(1);
  writeFileSync(
    `${process.env.E2E_EVIDENCE_DIR}/result.json`,
    JSON.stringify(
      {
        saved,
        updated,
        realApi: true,
        officialCli: "2.4.2",
        download: file.suggestedFilename(),
        immutableOnReload: true,
        explicitReimport: true,
      },
      null,
      2,
    ),
  );
});
