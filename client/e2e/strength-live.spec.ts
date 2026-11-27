import { test, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';
const ctx=JSON.parse(process.env.E2E_STRENGTH_CONTEXT ?? '{}');
const out=process.env.E2E_EVIDENCE_DIR!;

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
