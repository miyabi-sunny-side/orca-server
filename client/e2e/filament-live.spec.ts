import {expect,test} from '@playwright/test';
import {mkdirSync} from 'node:fs';
import {join} from 'node:path';

test('material CRUD, temperatures, manual mapping and stale observations',async({page,request})=>{
  test.setTimeout(120000);
  const c=JSON.parse(process.env.E2E_FILAMENT_CONTEXT!);const output=process.env.E2E_EVIDENCE_DIR!;mkdirSync(output,{recursive:true});
  let failList=true;
  await page.route('**/api/filament-products',route=>failList?route.fulfill({status:503,json:{error:'unavailable'}}):route.continue());
  await page.goto('/filaments');await expect(page.getByRole('alert')).toContainText('保存先');
  failList=false;await page.getByRole('button',{name:'読み直す',exact:true}).click();
  await expect(page.getByRole('link',{name:/PLA Matte 黒/})).toBeVisible();
  await page.getByRole('link',{name:'製品を追加',exact:true}).click();
  const name='温度調整用の長い製品名・試験フィラメント';
  await page.getByLabel('製品名',{exact:true}).fill(name);
  await page.getByLabel('メーカー',{exact:true}).fill('Example vendor');await page.getByLabel('材料種別').fill('PETG');
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(/\/filaments\/[0-9a-f-]+$/);
  const id=page.url().split('/').pop()!;
  for(const color of ['黒','白','黄']) {
    await page.getByRole('link',{name:'色を追加',exact:true}).click();
    await page.getByLabel('基本色').selectOption({label:color});
    await expect(page.getByLabel('色名',{exact:true})).toHaveValue(color);
    await expect(page.getByLabel('メーカー',{exact:true})).toHaveCount(0);
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(new RegExp(`/filaments/${id}$`));
  }
  await expect(page.getByRole('link',{name:'黒',exact:true})).toBeVisible();
  await page.getByRole('link',{name:'設定を追加'}).click();await page.getByLabel('機種・ノズル径').selectOption(c.machine);
  await page.getByLabel('基本のフィラメントプロファイル').selectOption('Generic PETG');
  await page.getByLabel('初層（℃）',{exact:true}).fill('250');await page.getByLabel('通常層（℃）',{exact:true}).fill('240');
  await page.getByLabel('ベッド初層（℃）',{exact:true}).fill('65');await page.getByLabel('ベッド通常層（℃）',{exact:true}).fill('0');
  let failSave=true;await page.route(`**/api/filament-products/${id}/settings`,route=>failSave?route.fulfill({status:409,json:{error:'duplicate'}}):route.continue());
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('alert')).toContainText('同じ機種');await expect(page.getByLabel('初層（℃）',{exact:true})).toHaveValue('250');
  failSave=false;
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(new RegExp(`/filaments/${id}$`));
  await expect(page.getByRole('link',{name:/初層 250℃ \/ 通常 240℃/})).toBeVisible();
  await page.getByRole('link',{name:/初層 250℃ \/ 通常 240℃/}).click();await page.getByLabel('初層（℃）',{exact:true}).fill('251');await page.getByRole('button',{name:'保存',exact:true}).click();
  await expect(page.getByRole('link',{name:/初層 251℃/})).toBeVisible();
  let product=await (await request.get(`/api/filament-products/${id}`)).json();
  for(const color of product.colors) {
    const detail=await (await request.get(`/api/filaments/${color.id}`)).json();
    expect(detail.settings[0].overrides_json).toEqual({nozzle_temperature_initial_layer:251,nozzle_temperature:240,bed_temperature_initial_layer:65,bed_temperature:0});
  }
  for(const scheme of ['dark','light'] as const){
    await page.emulateMedia({colorScheme:scheme});
    for(const width of [320,375,900]){
      await page.setViewportSize({width,height:812});await page.evaluate(()=>window.scrollTo(0,0));
      await page.screenshot({path:join(output,`product-${scheme}-${width}.png`),fullPage:true});
      expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
      expect(await page.locator('.primary').count()).toBe(1);
    }
  }
  await page.setViewportSize({width:320,height:812});
  await page.evaluate(()=>{const sizes=Array.from(document.querySelectorAll<HTMLElement>('body,body *')).map(el=>[el,parseFloat(getComputedStyle(el).fontSize)] as const);for(const[el,size]of sizes)el.style.fontSize=`${size*2}px`;});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.getByRole('link',{name:'色を追加',exact:true}).focus();await expect(page.getByRole('link',{name:'色を追加',exact:true})).toBeFocused();
  await page.screenshot({path:join(output,'product-text-200.png'),fullPage:true});await page.reload();
  await page.getByRole('link',{name:'黒',exact:true}).click();await page.getByText('正確な色を編集',{exact:true}).click();
  await page.getByLabel('色（RGBA・8桁）').fill('161616FF');await page.getByLabel('色名',{exact:true}).fill('チャコール');
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('link',{name:'チャコール',exact:true})).toBeVisible();
  product=await (await request.get(`/api/filament-products/${id}`)).json();expect(product.colors.find((x:any)=>x.name==='チャコール').color).toBe('161616FF');
  await page.getByRole('link',{name:/初層 251℃/}).click();await page.getByLabel('ベッド初層（℃）',{exact:true}).fill('');await page.getByLabel('ベッド通常層（℃）',{exact:true}).fill('');
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(new RegExp(`/filaments/${id}$`));
  product=await (await request.get(`/api/filament-products/${id}`)).json();expect(product.settings[0].overrides_json.bed_temperature).toBeUndefined();
  await page.getByRole('link',{name:/初層 251℃/}).click();page.once('dialog',d=>d.accept());await page.getByRole('button',{name:'この設定を削除'}).click();
  await expect(page.getByText('使う機種・ノズル径ごとに基本プロファイルを選択してください。')).toBeVisible();
  await page.getByText('製品の管理',{exact:true}).click();page.once('dialog',d=>d.accept());await page.getByRole('button',{name:'製品を削除',exact:true}).click();await expect(page).toHaveURL(/\/filaments$/);
  expect((await request.get(`/api/filament-products/${id}`)).status()).toBe(404);
  await page.goto(`/filaments/${c.gf}`);await page.getByText('製品の管理',{exact:true}).click();page.once('dialog',d=>d.accept());await page.getByRole('button',{name:'製品を削除',exact:true}).click();await expect(page.getByRole('alert')).toContainText('参照');
  await page.goto(`/printers/${c.printer}/ams`);
  const first=page.getByRole('listitem').filter({has:page.getByRole('button',{name:'AMS 0 スロット 1の詳細',exact:true})});
  const selector=first.getByRole('button',{name:/材料を選択/});
  const gfOption=first.getByRole('button',{name:/ガラス繊維入りPETG.*Third party/});
  await expect(selector).toContainText('ガラス繊維入りPETG');
  await first.getByRole('button',{name:'AMS 0 スロット 1の詳細',exact:true}).click();
  await expect(first.getByText('設定温度: 初層 250℃ / 通常 240℃')).toBeVisible();
  await expect(first.getByText(/ノズル.*(非対応|未確認|適合)/)).toHaveCount(0);
  for(const scheme of ['dark','light'] as const){
    await page.emulateMedia({colorScheme:scheme});
    for(const width of [320,375,900]){
      await page.setViewportSize({width,height:812});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
      await page.screenshot({path:join(output,`ams-${scheme}-${width}.png`),fullPage:true});
    }
  }
  await expect(first.getByText('不明',{exact:true}).first()).toBeVisible();
  await selector.click();await first.getByRole('button',{name:'指定を解除',exact:true}).click();await expect(selector).toContainText('材料は未指定');
  await selector.click();await gfOption.click();await expect(selector).toContainText('ガラス繊維入りPETG');
  await selector.click();await request.post(c.control,{data:{id:'0',tray_color:'000000FF'}});
  await expect(first.getByRole('alert')).toContainText('観測情報が変わりました',{timeout:12000});await expect(gfOption).toBeDisabled();
  await request.post(c.control,{data:{full:true}});await page.getByRole('button',{name:'状態を更新'}).click();await first.getByRole('button',{name:'キャンセル'}).click();
  await expect.poll(async()=>((await (await request.get(`/api/printers/${c.printer}/ams`)).json()).slots[0].reported.color)).toBe('FFFFFFFF');
  await page.getByRole('button',{name:'状態を更新'}).click();await selector.click();await gfOption.click();
  await expect(selector).toContainText('ガラス繊維入りPETG');
  await request.post(c.control,{data:{disconnect:true}});await expect(page.getByRole('status')).toContainText('現在の装填状態は未確認',{timeout:12000});
  await page.setViewportSize({width:375,height:812});await page.screenshot({path:join(output,'ams-disconnected.png'),fullPage:true});
  await selector.click();await expect(gfOption).toBeDisabled();await expect(first.getByRole('button',{name:'指定を解除',exact:true})).toBeEnabled();
  await first.getByRole('button',{name:'キャンセル'}).click();await request.post(c.control,{data:{full:true}});
  await expect.poll(async()=>((await (await request.get(`/api/printers/${c.printer}/ams`)).json()).current),{timeout:12000}).toBe(true);
  await page.getByRole('button',{name:'状態を更新'}).click();
  await page.evaluate(()=>{const sizes=Array.from(document.querySelectorAll<HTMLElement>('body,body *')).map(el=>[el,parseFloat(getComputedStyle(el).fontSize)] as const);for(const[el,size]of sizes)el.style.fontSize=`${size*2}px`;});
  expect(await page.locator('body').evaluate(el=>getComputedStyle(el).fontSize)).toBe('32px');expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await selector.focus();await expect(selector).toBeFocused();await page.evaluate(()=>window.scrollTo(0,0));
  await page.screenshot({path:join(output,'ams-text-200.png'),fullPage:true});
  await page.goto(`/filaments/${c.black}`);await page.getByText('既存の色をまとめる',{exact:true}).click();
  await page.getByRole('combobox',{name:'既存の色',exact:true}).selectOption(c.white);await page.getByRole('button',{name:'この製品にまとめる',exact:true}).click();
  await expect(page.getByRole('link',{name:'PLA Matte 白',exact:true})).toBeVisible();
  const merged=await (await request.get(`/api/filaments/${c.white}`)).json();expect(merged.product_id).toBe(c.black);
  await page.goto(`/filaments/${c.white}`);await expect(page).toHaveURL(new RegExp(`/filaments/${c.black}/colors/${c.white}$`));
  await expect(page.getByLabel('色名',{exact:true})).toHaveValue('PLA Matte 白');

});
