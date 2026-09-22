import { test, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';
const ctx=JSON.parse(process.env.E2E_STRENGTH_CONTEXT ?? '{}');
const out=process.env.E2E_EVIDENCE_DIR!;

test('details stay optional, persist strength and calculate shells in both themes',async({page,request})=>{
  test.setTimeout(90_000);mkdirSync(out,{recursive:true});
  for(const [width,colorScheme] of [[320,'dark'],[375,'dark'],[375,'light'],[900,'dark'],[900,'light']] as const){
    await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme});
    await page.goto('/plates/new');await page.getByRole('checkbox').check();await page.getByRole('button',{name:'構成を確認（1）'}).click();
    const details=page.locator('details').filter({has:page.locator('summary',{hasText:'詳細設定'})});
    await expect(details).not.toHaveAttribute('open');
    await page.getByLabel('プレート名',{exact:true}).fill(`充填・壁設定 ${width}`);
    await page.screenshot({path:`${out}/closed-${width}-${colorScheme}.png`,fullPage:true});
    await details.locator('summary').focus();await page.keyboard.press('Enter');await expect(details).toHaveAttribute('open');
    const walls=page.getByLabel('壁の枚数（周）'),density=page.getByLabel('充填率（%）'),pattern=page.getByRole('combobox',{name:'インフィル',exact:true});
    await expect(page.getByLabel('ブリムを付ける')).not.toBeChecked();await page.getByLabel('ブリムを付ける').check();
    await expect(pattern).toHaveValue('adaptivecubic');await expect(density).toHaveValue('15');await expect(walls).toHaveValue('2');
    await expect(pattern.locator('option')).toHaveCount(27);
    await expect(details.getByRole('status')).toContainText('上面5層・底面3層');
    await walls.fill('4');await expect(details.getByRole('status')).toContainText('上面10層・底面6層');
    await walls.fill('2');await expect(details.getByRole('status')).toContainText('上面5層・底面3層');
    await walls.fill('3');await expect(details.getByRole('status')).toContainText('上面8層・底面5層');
    await page.getByLabel('工程（品質）').selectOption('0.16mm Fixture quality');
    await expect(details.getByRole('status')).toContainText('上面11層・底面6層');
    await pattern.selectOption('gyroid');await density.fill('22.5');
    await expect(details.getByRole('status')).toContainText('上面11層・底面6層');
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`${out}/open-${width}-${colorScheme}.png`,fullPage:true});
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);
    const id=new URL(page.url()).pathname.split('/')[2];
    expect((await (await request.get('/api/plates/'+id)).json()).conditions).toMatchObject({sparse_infill_pattern:'gyroid',sparse_infill_density:22.5,wall_loops:3,brim_enabled:true});
    const add=page.getByRole('button',{name:'印刷キューへ',exact:true});
    expect(await add.evaluate(e=>e.getBoundingClientRect().width<e.parentElement!.getBoundingClientRect().width)).toBe(true);
    await page.reload();await page.getByRole('button',{name:'構成を編集'}).click();
    await expect(details).not.toHaveAttribute('open');await details.locator('summary').click();
    await expect(pattern).toHaveValue('gyroid');await expect(density).toHaveValue('22.5');await expect(walls).toHaveValue('3');
    await expect(page.getByLabel('ブリムを付ける')).toBeChecked();await page.getByLabel('ブリムを付ける').uncheck();
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('button',{name:'構成を編集'})).toBeVisible();
    expect((await (await request.get('/api/plates/'+id)).json()).conditions.brim_enabled).toBe(false);
    await page.addStyleTag({content:':root {--fs-xs:24px;--fs-sm:28px;--fs-md:30px;--fs-lg:32px;--fs-xl:34px;}'});
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  }
});

test('late defaults and failed saves preserve manual input; legacy values come from the process',async({page,request})=>{
  let release!:()=>void;const held=new Promise<void>(resolve=>release=resolve);
  await page.route('**/api/default-settings*',async route=>{await held;await route.continue();});
  await page.goto('/plates/new');await page.getByRole('checkbox').check();await page.getByRole('button',{name:'構成を確認（1）'}).click();
  await page.locator('summary',{hasText:'詳細設定'}).click();
  await page.getByLabel('壁の枚数（周）').fill('4');await page.getByLabel('充填率（%）').fill('0');await page.getByLabel('ブリムを付ける').check();
  release();await expect(page.getByRole('combobox',{name:'インフィル',exact:true})).toHaveValue('adaptivecubic');
  await expect(page.getByLabel('壁の枚数（周）')).toHaveValue('4');await expect(page.getByLabel('充填率（%）')).toHaveValue('0');
  await page.unroute('**/api/default-settings*');
  await page.getByLabel('プレート名',{exact:true}).fill('入力を保持');
  await page.route('**/api/plates/import',route=>route.fulfill({status:503,contentType:'application/json',body:'{}'}));
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('alert')).toBeVisible();
  await expect(page.getByLabel('プレート名',{exact:true})).toHaveValue('入力を保持');await expect(page.getByLabel('ブリムを付ける')).toBeChecked();
  await expect(page.getByLabel('壁の枚数（周）')).toHaveValue('4');await expect(page.getByLabel('充填率（%）')).toHaveValue('0');
  await page.unroute('**/api/plates/import');await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);
  const before=await (await request.get('/api/plates/'+ctx.legacy)).json();expect(before.conditions.wall_loops).toBeNull();
  await page.goto('/plates/'+ctx.legacy);await page.getByRole('button',{name:'構成を編集'}).click();await page.locator('summary',{hasText:'詳細設定'}).click();
  await expect(page.getByRole('combobox',{name:'インフィル',exact:true})).toHaveValue('crosshatch');await expect(page.getByLabel('壁の枚数（周）')).toHaveValue('2');
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('button',{name:'構成を編集'})).toBeVisible();
  expect((await (await request.get('/api/plates/'+ctx.legacy)).json()).conditions).toMatchObject({sparse_infill_pattern:'crosshatch',sparse_infill_density:15,wall_loops:2});
});

test('saved defaults only affect new plates and survive reload',async({page,request})=>{
  const before=await (await request.get('/api/plates/'+ctx.legacy)).json();
  await page.goto('/printers');const summary=page.locator('summary',{hasText:'新規プレートの詳細初期値'});
  await summary.focus();await page.keyboard.press('Space');
  await page.getByRole('combobox',{name:'インフィル',exact:true}).selectOption('gyroid');await page.getByLabel('壁の枚数（周）').fill('4');await page.getByLabel('充填率（%）').fill('25');
  await page.getByRole('button',{name:'初期値を保存',exact:true}).click();await expect(page.getByText('新規プレートの初期値を保存しました。',{exact:true})).toBeVisible();
  await page.reload();await summary.click();await expect(page.getByLabel('壁の枚数（周）')).toHaveValue('4');
  await page.screenshot({path:`${out}/defaults.png`,fullPage:true});
  expect(await (await request.get('/api/plates/'+ctx.legacy)).json()).toEqual(before);
  await page.goto('/plates/new');await page.getByRole('checkbox').check();await page.getByRole('button',{name:'構成を確認（1）'}).click();await page.locator('summary',{hasText:'詳細設定'}).click();
  await expect(page.getByRole('combobox',{name:'インフィル',exact:true})).toHaveValue('gyroid');await expect(page.getByLabel('壁の枚数（周）')).toHaveValue('4');await expect(page.getByLabel('充填率（%）')).toHaveValue('25');
});
