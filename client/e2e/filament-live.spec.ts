import {expect,test} from '@playwright/test';
import {mkdirSync} from 'node:fs';
import {join} from 'node:path';

test('material CRUD, temperatures, manual mapping and stale observations',async({page,request})=>{
  test.setTimeout(120000);
  const c=JSON.parse(process.env.E2E_FILAMENT_CONTEXT!);const output=process.env.E2E_EVIDENCE_DIR!;mkdirSync(output,{recursive:true});
  await page.route('**/api/filaments',route=>route.fulfill({status:503,json:{error:'unavailable'}}));
  await page.goto('/filaments');await expect(page.getByRole('alert')).toContainText('保存先');
  await expect(page.getByText('使用するフィラメントを銘柄と色ごとに登録してください。AMSのスロットと対応づけられます。')).toHaveCount(0);
  await page.unroute('**/api/filaments');await page.getByRole('button',{name:'読み直す',exact:true}).click();
  await expect(page.getByRole('link',{name:/PLA Matte 黒/})).toBeVisible();await expect(page.getByRole('link',{name:/PLA Matte 白/})).toBeVisible();
  await page.getByRole('link',{name:'追加',exact:true}).click();
  await page.getByLabel('材料名',{exact:true}).fill('温度調整用の長い材料名・透明ブルーの試験フィラメント');
  await page.getByLabel('メーカー・銘柄').fill('Example vendor');await page.getByLabel('材料種別').fill('PETG');await page.getByLabel('色（RGBA・8桁）').fill('00AAFFFF');
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(/\/filaments\/[0-9a-f-]+$/);
  const id=page.url().split('/').pop()!;
  await page.getByLabel('材料名',{exact:true}).fill('温度調整用の長い材料名・透明ブルーの試験フィラメント（更新）');
  await Promise.all([page.waitForEvent('framenavigated',{predicate:frame=>frame===page.mainFrame()}),page.getByRole('button',{name:'保存',exact:true}).click()]);
  await page.waitForLoadState('load');await expect(page.getByLabel('材料名',{exact:true})).toHaveValue('温度調整用の長い材料名・透明ブルーの試験フィラメント（更新）');
  await page.getByRole('link',{name:'設定を追加'}).click();await page.getByLabel('機種・ノズル径').selectOption(c.machine);
  await page.getByLabel('基本のフィラメントプロファイル').selectOption('Generic PETG');
  await page.getByLabel('初層（℃）',{exact:true}).fill('250');await page.getByLabel('通常層（℃）',{exact:true}).fill('240');
  await page.route(`**/api/filaments/${id}/settings`,route=>route.fulfill({status:409,json:{error:'duplicate'}}));
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('alert')).toContainText('同じ機種');await expect(page.getByLabel('初層（℃）',{exact:true})).toHaveValue('250');
  await page.unroute(`**/api/filaments/${id}/settings`);
  for(const scheme of ['dark','light'] as const){
    await page.emulateMedia({colorScheme:scheme});await page.setViewportSize({width:375,height:812});
    await page.screenshot({path:join(output,`temperature-${scheme}.png`),fullPage:true});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  }
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(new RegExp(`/filaments/${id}$`));
  await expect(page.getByRole('link',{name:/初層 250℃ \/ 通常 240℃/})).toBeVisible();
  await page.getByRole('link',{name:/初層 250℃ \/ 通常 240℃/}).click();await page.getByLabel('初層（℃）',{exact:true}).fill('251');await page.getByRole('button',{name:'保存',exact:true}).click();
  await expect(page.getByRole('link',{name:/初層 251℃/})).toBeVisible();
  await page.getByRole('link',{name:/初層 251℃/}).click();page.once('dialog',d=>d.accept());await page.getByRole('button',{name:'この設定を削除'}).click();
  await expect(page.getByText('使う機種・ノズル径ごとに基本プロファイルを選択してください。')).toBeVisible();
  page.once('dialog',d=>d.accept());await page.getByRole('button',{name:'材料を削除',exact:true}).click();await expect(page).toHaveURL(/\/filaments$/);
  expect((await request.get(`/api/filaments/${id}`)).status()).toBe(404);
  await page.goto(`/filaments/${c.gf}`);page.once('dialog',d=>d.accept());await page.getByRole('button',{name:'材料を削除',exact:true}).click();await expect(page.getByRole('alert')).toContainText('参照');
  await page.goto(`/printers/${c.printer}/ams`);
  const first=page.getByRole('listitem').filter({has:page.getByRole('heading',{name:'AMS 0 · スロット 1',exact:true})});
  await expect(first.getByText('ガラス繊維入りPETG',{exact:true})).toBeVisible();await expect(first.getByText('設定温度: 初層 250℃ / 通常 240℃')).toBeVisible();
  for(const scheme of ['dark','light'] as const){
    await page.emulateMedia({colorScheme:scheme});
    for(const width of [320,375,900]){
      await page.setViewportSize({width,height:812});expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
      await page.screenshot({path:join(output,`ams-${scheme}-${width}.png`),fullPage:true});
    }
  }
  await first.getByText('プリンターからの報告',{exact:true}).click();await expect(first.getByText('不明',{exact:true}).first()).toBeVisible();
  await first.getByRole('button',{name:'材料を指定・解除'}).click();await first.getByLabel('対応する材料').selectOption('');await first.getByRole('button',{name:'対応を保存'}).click();await expect(first.getByText('材料は未指定',{exact:true})).toBeVisible();
  await first.getByRole('button',{name:'材料を指定・解除'}).click();await first.getByLabel('対応する材料').selectOption(c.gf);await first.getByRole('button',{name:'対応を保存'}).click();await expect(first.getByText('ガラス繊維入りPETG',{exact:true})).toBeVisible();
  await first.getByRole('button',{name:'材料を指定・解除'}).click();await request.post(c.control,{data:{id:'0',tray_color:'000000FF'}});
  await expect(first.getByRole('alert')).toContainText('観測情報が変わりました',{timeout:12000});await expect(first.getByRole('button',{name:'対応を保存'})).toBeDisabled();
  await request.post(c.control,{data:{full:true}});await page.getByRole('button',{name:'状態を更新'}).click();await first.getByRole('button',{name:'キャンセル'}).click();
  await expect.poll(async()=>((await (await request.get(`/api/printers/${c.printer}/ams`)).json()).slots[0].reported.color)).toBe('FFFFFFFF');
  await page.getByRole('button',{name:'状態を更新'}).click();await first.getByRole('button',{name:'材料を指定・解除'}).click();await first.getByLabel('対応する材料').selectOption(c.gf);await first.getByRole('button',{name:'対応を保存'}).click();
  await expect(first.getByText('ガラス繊維入りPETG',{exact:true})).toBeVisible();
  await request.post(c.control,{data:{disconnect:true}});await expect(page.getByRole('status')).toContainText('現在の装填状態は未確認',{timeout:12000});
  await page.setViewportSize({width:375,height:812});await page.screenshot({path:join(output,'ams-disconnected.png'),fullPage:true});
  await first.getByRole('button',{name:'材料を指定・解除'}).click();await expect(first.getByRole('button',{name:'対応を保存'})).toBeDisabled();await first.getByLabel('対応する材料').selectOption('');await expect(first.getByRole('button',{name:'対応を保存'})).toBeEnabled();
  await first.getByRole('button',{name:'キャンセル'}).click();await request.post(c.control,{data:{full:true}});
  await expect.poll(async()=>((await (await request.get(`/api/printers/${c.printer}/ams`)).json()).current),{timeout:12000}).toBe(true);
  await page.getByRole('button',{name:'状態を更新'}).click();
  await page.evaluate(()=>{const sizes=Array.from(document.querySelectorAll<HTMLElement>('body,body *')).map(el=>[el,parseFloat(getComputedStyle(el).fontSize)] as const);for(const[el,size]of sizes)el.style.fontSize=`${size*2}px`;});
  expect(await page.locator('body').evaluate(el=>getComputedStyle(el).fontSize)).toBe('32px');expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await first.getByRole('button',{name:'材料を指定・解除'}).focus();await expect(first.getByRole('button',{name:'材料を指定・解除'})).toBeFocused();
  await page.evaluate(()=>window.scrollTo(0,0));
  await page.screenshot({path:join(output,'ams-text-200.png'),fullPage:true});
});
