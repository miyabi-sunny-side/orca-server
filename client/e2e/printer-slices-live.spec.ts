import {selectMaterial} from './plate-material';
import {test,expect} from '@playwright/test';
import {mkdirSync} from 'node:fs';
const ctx=JSON.parse(process.env.E2E_PRINTER_SLICES_CONTEXT ?? '{}');
test('a plate without machine conditions shows each printer, refuses the bed it does not fit and starts on the P1S',async({page,request})=>{
  test.setTimeout(300_000);page.setDefaultTimeout(15_000);
  const out=process.env.E2E_EVIDENCE_DIR!;mkdirSync(out,{recursive:true});
  for(const [width,colorScheme] of [[320,'dark'],[900,'light']] as const){
    await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme});
    await page.goto('/plates/new');
    await page.getByLabel(ctx.model,{exact:true}).check();await page.getByRole('button',{name:'構成を確認（1）'}).click();
    for(const name of ['要求する機種・ノズル','工程（品質）','ビルドプレート'])await expect(page.getByLabel(name,{exact:true})).toHaveCount(0);
    await page.getByLabel('プレート名',{exact:true}).fill(`大きなトレイ ${width}`);
    await selectMaterial(page,ctx.material);
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`${out}/create-${width}-${colorScheme}.png`,fullPage:true});
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);
    const rows=page.getByLabel('プリンターごとの試算').getByRole('listitem');
    // The default printer is chosen first, not the first printer ID.
    await expect(page.getByLabel('追加先のプリンター')).toHaveValue('p1');
    await expect(rows).toHaveCount(2);
    await expect(rows.filter({hasText:'A1 mini'})).toContainText('A1 mini · 台に乗りません',{timeout:180_000});
    await expect(rows.filter({hasText:'P1S'})).toContainText(/P1S · 約\d/,{timeout:180_000});
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`${out}/detail-${width}-${colorScheme}.png`,fullPage:true});
    await rows.filter({hasText:'A1 mini'}).locator('summary').click();
    await expect(rows.filter({hasText:'A1 mini'}).getByRole('alert')).toContainText('台に乗りません');
    // The A1 mini cannot take the plate; the P1S can.
    const target=page.getByLabel('追加先のプリンター');const add=page.getByRole('button',{name:'印刷キューへ',exact:true});
    await target.selectOption(ctx.mini);
    await expect(page.getByText('このプリンターの台に乗りません',{exact:false}).last()).toBeVisible();await expect(add).toBeDisabled();
    await target.selectOption('p1');await expect(add).toBeEnabled();await add.click();
    await expect(page.getByRole('status').filter({hasText:'キューに追加しました'})).toBeVisible();
    await page.addStyleTag({content:':root {--fs-xs:24px;--fs-sm:28px;--fs-md:30px;--fs-lg:32px;--fs-xl:34px;} html {font-size:200% !important}'});
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`${out}/detail-200-${width}-${colorScheme}.png`,fullPage:true});
  }
  // Start the first waiting plate from the queue page.
  await page.goto('/queue?printer_id=p1');
  const confirm=page.getByRole('checkbox',{name:'造形物を取り外し、空のビルドプレートを戻しました'});
  if(await confirm.count())await confirm.check();
  await page.getByRole('button',{name:/^(印刷|次を印刷)$/}).click();
  await expect.poll(async()=>(await (await request.get('/api/queue?printer_id=p1')).json()).current?.state,{timeout:120_000}).toMatch(/preparing|printing/);
  await page.screenshot({path:`${out}/queue-started.png`,fullPage:true});
});
