import {selectMaterial} from './plate-material';
import { test, expect } from '@playwright/test';
import { mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
const context = JSON.parse(process.env.E2E_IMPORT_CONTEXT ?? '{}');
for (const width of [375, 1000]) for (const colorScheme of ['dark', 'light'] as const) {
  test(`external models: ${width}px ${colorScheme}, preserve files and recover without SCAD`, async ({ page, request }) => {
    test.setTimeout(120_000);page.setDefaultTimeout(15_000);
    const directory = process.env.E2E_EVIDENCE_DIR!;mkdirSync(directory,{recursive:true});
    await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme});
    const scad: string[] = [];page.on('request',r=>{if(r.url().includes('/api/scad'))scad.push(r.url());});
    const queueBefore = await (await request.get('/api/queue?printer_id=p1')).json();
    await page.goto('/plates');
    await page.getByRole('link',{name:'ファイルから取り込む',exact:true}).focus();await page.keyboard.press('Enter');
    await expect(page).toHaveURL(/source=file/);
    const input=page.getByLabel('モデルファイル');
    await input.setInputFiles(join(context.fixtures,'bad.3mf'));
    await expect(page.getByRole('alert')).toContainText('3MF');
    await page.getByRole('button',{name:'再試行',exact:true}).click();await expect(page.getByRole('alert')).toBeVisible();
    await input.setInputFiles(join(context.fixtures,'multiple.3mf'));
    const selection=page.getByLabel('取り込むプレート');await expect(selection).toBeVisible();
    await selection.focus();await page.keyboard.press('ArrowDown');await page.keyboard.press('Tab');
    await expect(selection).toHaveValue('1');
    const preview=page.getByRole('complementary',{name:'STLプレビュー'});
    await expect(preview.locator('.dimensions')).toHaveText('23.0 × 20.0 × 20.0 mm');
    const name=`外部モデル ${width} ${colorScheme}`;await page.getByLabel('プレート名',{exact:true}).fill(name);
    await page.getByLabel('寸法確認 2 の個数').fill('2');
    const material=(await (await request.get('/api/filaments')).json())[0];
    await selectMaterial(page,material.id);
    const save=page.getByRole('button',{name:'保存',exact:true});await expect(save).toBeEnabled();
    let failed=false;
    await page.route('**/api/plates/files',async route=>{if(!failed){failed=true;await route.fulfill({status:503,json:{error:'Fixture temporary failure'}});}else await route.continue();});
    await save.click();await expect(page.getByRole('alert')).toBeVisible();
    await expect(page.getByLabel('プレート名',{exact:true})).toHaveValue(name);
    await expect(page.getByLabel('寸法確認 2 の個数')).toHaveValue('2');
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.evaluate(()=>window.scrollTo(0,0));await page.screenshot({path:join(directory,`import-${width}-${colorScheme}.png`),fullPage:true});
    await save.focus();await page.keyboard.press('Enter');await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);
    await page.reload();await expect(page.getByRole('heading',{level:1})).toHaveText(name);
    await expect(preview.locator('.dimensions')).toHaveText('23.0 × 20.0 × 20.0 mm');
    const saved=await (await request.get('/api'+new URL(page.url()).pathname)).json();
    expect(saved.conditions.filament_id).toBe(material.id);
    expect(saved.models[0].quantity).toBe(2);expect(saved.imported.selection.items[0].build_index).toBe(1);
    const original=await request.get((await page.getByRole('link',{name:'元の3MFを取得',exact:true}).getAttribute('href'))!);
    expect(await original.body()).toEqual(readFileSync(join(context.fixtures,'multiple.3mf')));
    const derived=await request.get((await page.getByRole('link',{name:'確認用STLを取得',exact:true}).getAttribute('href'))!);
    const derivedBytes=await derived.body();expect(derivedBytes.length).toBe(84+24*50);expect(derivedBytes.readUInt32LE(80)).toBe(24);
    await expect(page.getByRole('link',{name:'アップロードした元STLを取得',exact:true})).toHaveCount(0);
    await page.getByRole('button',{name:'構成を編集'}).click();await page.getByLabel('プレート名',{exact:true}).fill(name+' 編集');await save.click();
    await expect(page.getByRole('heading',{level:1})).toHaveText(name+' 編集');
    expect((await (await request.get('/api'+new URL(page.url()).pathname)).json()).imported).toEqual(saved.imported);
    await page.goto('/plates/new?source=file');await input.setInputFiles(join(context.fixtures,'painted.3mf'));
    await expect(page.getByText('多色・ペイントの印刷は未対応です。元の3MFはそのまま保存できます。',{exact:true})).toBeVisible();
    await expect(preview.locator('.dimensions')).toHaveText('23.0 × 20.0 × 20.0 mm');await save.click();
    await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);await expect(page.getByRole('button',{name:'印刷キューへ',exact:true})).toBeDisabled();
    await expect(page.getByRole('link',{name:'元の3MFを取得',exact:true})).toBeVisible();
    await expect(page.getByRole('link',{name:'AMSを確認',exact:true})).toHaveCount(0);
    await page.evaluate(()=>window.scrollTo(0,0));await page.screenshot({path:join(directory,`saved-color-${width}-${colorScheme}.png`),fullPage:true});
    await page.goto('/plates/new?source=file');await input.setInputFiles(join(context.fixtures,'single.3mf'));
    await expect(preview.locator('.dimensions')).toHaveText('23.0 × 20.0 × 20.0 mm');await expect(selection).toHaveCount(0);
    await input.setInputFiles([
      {name:'一つ目のとても長いファイル名を含む形状の確認.stl',mimeType:'model/stl',buffer:readFileSync(join(context.fixtures,'cube.stl'))},
      {name:'second.stl',mimeType:'model/stl',buffer:readFileSync(join(context.fixtures,'cube.stl'))},
    ]);
    await expect(preview.locator('.dimensions')).toHaveText('20.0 × 20.0 × 20.0 mm');
    await page.getByLabel('second.stl の個数').fill('3');
    const removeFirst = width === 375 && colorScheme === 'dark';
    if (removeFirst) {
      await page.getByRole('button', {name:'一つ目のとても長いファイル名を含む形状の確認.stlを構成から外す'}).click();
      await expect(preview.getByRole('heading')).toHaveText('second.stl');
      await expect(page.getByLabel('表示するモデル')).toHaveCount(0);
    }
    await save.click();await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);
    const stls=await (await request.get('/api'+new URL(page.url()).pathname)).json();expect(stls.models.map((m:any)=>m.quantity)).toEqual(removeFirst ? [3] : [1,3]);expect(stls.imported).toBeUndefined();
    await expect(page.getByRole('link',{name:'アップロードした元STLを取得',exact:true})).toHaveCount(removeFirst ? 1 : 2);
    expect(scad).toEqual([]);
    const queueAfter=await (await request.get('/api/queue?printer_id=p1')).json();expect(queueAfter.waiting.map((j:any)=>j.id)).toEqual(queueBefore.waiting.map((j:any)=>j.id));expect(queueAfter.current).toEqual(queueBefore.current);
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.addStyleTag({content:'html {font-size:200% !important}'});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  });
}
