import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { materialButton, selectMaterial, expectMaterial } from './plate-material';
const c=JSON.parse(process.env.E2E_FILAMENT_PICKER_CONTEXT ?? '{}');
const out=process.env.E2E_EVIDENCE_DIR!;
const allLabel='所持していないフィラメントを選択する';
const dialog=(page:Page)=>page.getByRole('dialog');
const option=(page:Page,id:string)=>dialog(page).locator(`button[data-filament-id="${id}"]`);
const search=(page:Page)=>dialog(page).getByRole('searchbox',{name:'材料を検索'});
const supportButton=(page:Page)=>page.getByRole('button',{name:/^接触面のフィラメント:/});
async function edit(page:Page) {
  await page.goto('/plates/'+c.plate);await page.getByRole('button',{name:'構成を編集'}).click();
}
async function fit(page:Page) {
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  expect(await dialog(page).evaluate(e=>e.scrollWidth<=e.clientWidth)).toBe(true);
}

test('loaded/all selection, keyboard and persistence at 320/375/900px in both themes',async({page,request})=>{
  test.setTimeout(180_000);mkdirSync(out,{recursive:true});
  await request.put('/api/default-settings',{data:{default_printer_id:'p1'}});
  for(const width of [320,375,900]) for(const colorScheme of ['dark','light'] as const) {
    await request.post(c.control,{data:{}});
    await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme});
    // The original plate gives the before/after comparison the same values and layout.
    await edit(page);await expectMaterial(page,c.white);
    await page.screenshot({path:`${out}/form-${width}-${colorScheme}.png`,fullPage:true});
    await page.goto('/plates/new');await page.getByRole('checkbox').check();await page.getByRole('button',{name:'構成を確認（1）'}).click();
    await expectMaterial(page,c.white);
    await page.getByLabel('プレート名',{exact:true}).fill(`Picker ${width} ${colorScheme}`);
    const main=materialButton(page);
    await main.focus();await page.keyboard.press('Enter');
    await expect(search(page)).toBeFocused();await expect(dialog(page).getByLabel(allLabel)).not.toBeChecked();
    await expect(dialog(page).locator('.choices button')).toHaveCount(3);
    await expect(option(page,c.mini_material)).toHaveCount(0);await expect(option(page,c.future)).toHaveCount(0);
    expect(await main.evaluate(e=>e.getBoundingClientRect().height)).toBeGreaterThanOrEqual(44);
    await search(page).fill('Fixture');await expect(dialog(page).locator('.choices button')).toHaveCount(3);
    await dialog(page).getByLabel(allLabel).check();await expect(search(page)).toHaveValue('Fixture');
    await expect(dialog(page).locator('.choices button')).toHaveCount(5);
    await dialog(page).getByLabel(allLabel).uncheck();await expect(search(page)).toHaveValue('Fixture');
    await expect(dialog(page).locator('.choices button')).toHaveCount(3);
    await fit(page);await page.screenshot({path:`${out}/modal-${width}-${colorScheme}.png`});
    // Tab is trapped in the modal; each closing path restores the initiating control.
    await dialog(page).getByRole('button',{name:'閉じる',exact:true}).focus();await page.keyboard.press('Shift+Tab');
    await expect(dialog(page).getByRole('button',{name:'指定を解除',exact:true})).toBeFocused();
    await page.keyboard.press('Tab');await expect(dialog(page).getByRole('button',{name:'閉じる',exact:true})).toBeFocused();
    await search(page).focus();await page.keyboard.press('Escape');await expect(main).toBeFocused();await expectMaterial(page,c.white);
    await main.click();await dialog(page).getByRole('button',{name:'キャンセル'}).click();await expect(main).toBeFocused();
    await main.click();await dialog(page).getByRole('button',{name:'閉じる',exact:true}).click();await expect(main).toBeFocused();
    await main.click();await page.locator('.scrim').click({position:{x:2,y:2}});await expect(main).toBeFocused();
    await main.click();await dialog(page).getByLabel(allLabel).check();await search(page).fill('GF');
    await expect(option(page,c.future)).toBeVisible();await search(page).press('ArrowDown');await expect(option(page,c.future)).toBeFocused();await page.keyboard.press('Enter');
    await expect(main).toBeFocused();await expect(main).toContainText('未装填');
    await main.click();await expect(dialog(page).getByLabel(allLabel)).not.toBeChecked();await expect(dialog(page).locator('.choices button')).toHaveCount(3);await expect(option(page,c.future)).toHaveCount(0);
    await page.keyboard.press('Escape');await expectMaterial(page,c.future);
    await page.locator('summary').filter({hasText:'詳細設定'}).click();await page.getByLabel('サポートを使う').check();
    await expect(supportButton(page)).toContainText('PETG-GF');
    await supportButton(page).click();await expect(search(page)).toBeFocused();await expect(dialog(page).locator('.choices button')).toHaveCount(3);
    await search(page).fill('PLA 青');await expect(option(page,c.blue)).toBeVisible();await option(page,c.blue).click();await expect(supportButton(page)).toBeFocused();
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);
    const id=new URL(page.url()).pathname.split('/')[2];const saved=await (await request.get('/api/plates/'+id)).json();
    expect(saved.conditions).toMatchObject({filament_id:c.future,support_interface_filament_id:c.blue,support_enabled:true});
    expect(saved.conditions).not.toHaveProperty('include_unloaded');await page.reload();await page.getByRole('button',{name:'構成を編集'}).click();
    await expectMaterial(page,c.future);await expect(main).toContainText('未装填');
    await page.locator('summary').filter({hasText:'詳細設定'}).click();await expect(supportButton(page)).toContainText('PLA 青');
    await supportButton(page).click();await dialog(page).getByRole('button',{name:'主材料と同じにする',exact:true}).click();
    await expect(supportButton(page)).toContainText('PETG-GF');await expect(supportButton(page)).toBeFocused();
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('button',{name:'構成を編集'})).toBeVisible();
    expect((await (await request.get('/api/plates/'+id)).json()).conditions.support_interface_filament_id).toBe(c.future);
    await page.getByRole('button',{name:'構成を編集'}).click();
    await main.click();await dialog(page).getByLabel(allLabel).check();await search(page).fill('長い');
    await expect(dialog(page).locator('.choices button')).toHaveCount(2);
    const fontBefore=await search(page).evaluate(e=>parseFloat(getComputedStyle(e).fontSize));
    await page.addStyleTag({content:':root {--fs-xs:24px;--fs-sm:28px;--fs-md:30px;--fs-lg:32px;--fs-xl:34px}'});
    expect(await search(page).evaluate(e=>parseFloat(getComputedStyle(e).fontSize))).toBe(fontBefore*2);
    expect(await search(page).evaluate(e=>e.getBoundingClientRect().width)).toBeGreaterThan(200);await fit(page);
    await option(page,c.future).scrollIntoViewIfNeeded();await expect(option(page,c.future)).toBeInViewport();
    await search(page).scrollIntoViewIfNeeded();await expect(search(page)).toBeInViewport();await dialog(page).getByLabel(allLabel).scrollIntoViewIfNeeded();await expect(dialog(page).getByLabel(allLabel)).toBeInViewport();
    await page.screenshot({path:`${out}/text200-${width}-${colorScheme}.png`});
    await page.keyboard.press('Escape');await expect(main).toBeFocused();
  }
});

test('late and failed responses retain query, scope and selection',async({page,request})=>{
  await request.post(c.control,{data:{}});await edit(page);
  let release!:()=>void, seen!:()=>void;
  const gate=new Promise<void>(r=>release=r), started=new Promise<void>(r=>seen=r);
  let failing=true;
  await page.route('**/api/plate-filaments?*',async route=>{
    const q=new URL(route.request().url()).searchParams.get('q');
    if(q==='GF') { const response=await route.fetch();seen();await gate;await route.fulfill({response}); }
    else if(q==='lost' && failing) await route.fulfill({status:503,json:{error:'Fixture unavailable'}});
    else await route.continue();
  });
  await materialButton(page).click();await dialog(page).getByLabel(allLabel).check();await search(page).fill('GF');await started;
  await search(page).fill('PLA 青');await expect(option(page,c.blue)).toBeVisible();release();
  await expect(search(page)).toHaveValue('PLA 青');await expect(option(page,c.future)).toHaveCount(0);
  await search(page).fill('lost');await expect(dialog(page).getByRole('alert')).toBeVisible();await expect(search(page)).toHaveValue('lost');await expect(dialog(page).getByLabel(allLabel)).toBeChecked();
  await page.screenshot({path:out+'/request-failed.png'});
  failing=false;await dialog(page).getByRole('button',{name:'検索をやり直す'}).click();await expect(dialog(page).getByRole('alert')).toHaveCount(0);await expect(search(page)).toHaveValue('lost');
  await expect(dialog(page).getByText('一致する材料がありません。',{exact:false})).toBeVisible();
  await search(page).press('Escape');await expectMaterial(page,c.white);
  await page.getByLabel('要求する機種・ノズル').selectOption(c.mini);await expectMaterial(page,c.white);await expect(materialButton(page)).toContainText('未装填');
  await materialButton(page).click();await expect(dialog(page).locator('.choices button')).toHaveCount(1);await expect(option(page,c.mini_material)).toBeVisible();await search(page).press('Escape');
  await page.getByLabel('要求する機種・ノズル').selectOption({label:'未設定'});await materialButton(page).click();await expect(dialog(page).locator('.choices button')).toHaveCount(4);
  await option(page,c.blue).click();await expect(page.getByLabel('要求する機種・ノズル')).toHaveValue('');
  await expectMaterial(page,c.blue);
});

test('unknown, unmapped, empty and missing references remain distinct; clear saves null',async({page,request})=>{
  test.setTimeout(60_000);await request.post(c.control,{data:{}});await edit(page);
  // Removing a mapping leaves physical material present, and offers the AMS correction path.
  const inventory=await (await request.get('/api/printers/p1/ams')).json();const slot=inventory.slots.find((s:any)=>s.slot_index===3);
  expect((await request.put('/api/printers/p1/ams/'+slot.id,{data:{revision:slot.revision,filament_id:null}})).status()).toBe(204);
  await materialButton(page).click();await expect(dialog(page).getByText(/未割当の材料/)).toBeVisible();await expect(dialog(page).getByRole('link',{name:'AMSの材料割当'})).toHaveAttribute('href','/printers/p1/ams');await expect(option(page,c.blue)).toHaveCount(0);
  await search(page).fill('PLA');await request.post(c.control,{data:{offline:true}});
  await expect.poll(async()=>{const r=await (await request.get('/api/plate-filaments')).json();return r.printers.every((p:any)=>p.state==='unconfirmed');}).toBe(true);
  await dialog(page).getByLabel(allLabel).check();await dialog(page).getByLabel(allLabel).uncheck();await expect(search(page)).toHaveValue('PLA');
  await expect(dialog(page).locator('.choices button')).toHaveCount(0);await expect(dialog(page).getByRole('button',{name:'装填情報を再確認'})).toBeVisible();await expect(dialog(page).getByLabel(allLabel)).not.toBeChecked();
  await page.screenshot({path:out+'/unconfirmed.png'});await search(page).press('Escape');await expect(materialButton(page)).toContainText('装填未確認');
  // Reconnect and supply an explicit empty report: this is now unloaded, not unknown.
  await request.post(c.control,{data:{empty:true}});
  await expect.poll(async()=>{await request.post(c.control,{data:{empty:true}});const r=await (await request.get('/api/plate-filaments')).json();return r.printers.every((p:any)=>p.state==='current') && r.filaments.length===0;}).toBe(true);
  await materialButton(page).click();await expect(dialog(page).locator('.choices button')).toHaveCount(0);await expect(dialog(page).getByRole('button',{name:'装填情報を再確認'})).toHaveCount(0);await search(page).press('Escape');await expect(materialButton(page)).toContainText('未装填');
  await selectMaterial(page,null);await expectMaterial(page,null);await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('button',{name:'構成を編集'})).toBeVisible();
  expect((await (await request.get('/api/plates/'+c.plate)).json()).conditions.filament_id).toBeNull();
  await page.reload();await page.getByRole('button',{name:'構成を編集'}).click();await expectMaterial(page,null);
  await selectMaterial(page,c.future);await expect(materialButton(page)).toContainText('未装填');
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('button',{name:'構成を編集'})).toBeVisible();
  expect((await (await request.get('/api/plates/'+c.plate)).json()).conditions.filament_id).toBe(c.future);
  await request.post(c.control,{data:{missing:true,plate_id:c.plate}});await edit(page);await expect(materialButton(page)).toContainText('登録が見つかりません');
  await materialButton(page).click();await dialog(page).getByLabel(allLabel).check();await expect(dialog(page).locator('.choices button')).toHaveCount(5);await expect(option(page,'missing-material')).toHaveCount(0);await search(page).press('Escape');
  await expect(materialButton(page)).toContainText('登録が見つかりません');
  await expect(page.getByLabel('工程（品質）').locator('option:checked')).not.toContainText('組合せを確認');
  await expect(page.getByText('工程が要求する機種・ノズルに対応していません。',{exact:false})).toHaveCount(0);
  await expect(page.getByRole('link',{name:'材料設定へ'})).toHaveCount(0);
  await page.screenshot({path:out+'/missing-reference.png',fullPage:true});
  await selectMaterial(page,c.future);await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('button',{name:'構成を編集'})).toBeVisible();
  expect((await (await request.get('/api/plates/'+c.plate)).json()).conditions.filament_id).toBe(c.future);
});
