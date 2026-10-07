import {test,expect,type Page} from '@playwright/test';
import {mkdirSync} from 'node:fs';
const ctx=JSON.parse(process.env.E2E_ARCHIVE_CONTEXT??'{}');
const out=process.env.E2E_EVIDENCE_DIR!;
const row=(page:Page,name:string)=>page.locator('.plate-row').filter({has:page.getByText(name,{exact:true})});
const menuButtons=(page:Page)=>page.getByRole('dialog').locator('.plate-menu > button, .plate-menu .queue-add button').allTextContents();
async function fit(page:Page){expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);}
async function loaded(page:Page,path:string){await page.goto(path);await expect(page.locator('section.content')).toHaveAttribute('data-state','success');}

test('right click, long press and the ContextMenu key archive and restore a plate',async({browser,request})=>{
  test.setTimeout(120_000);mkdirSync(out,{recursive:true});
  const context=await browser.newContext({viewport:{width:375,height:812},hasTouch:true});const page=await context.newPage();
  const cdp=await context.newCDPSession(page);const name=ctx.plate.name;
  const open=async(gesture:string,target:ReturnType<typeof row>)=>{
    if(gesture==='right')await target.click({button:'right'});
    else if(gesture==='key'){await target.focus();await target.press('ContextMenu');}
    else{const box=(await target.boundingBox())!;const x=box.x+60,y=box.y+20;
      await cdp.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x,y}]});
      await expect(page.getByRole('dialog')).toBeVisible();await cdp.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});}
    await expect(page.getByRole('dialog')).toBeVisible();
  };
  for(const gesture of ['right','touch','key']){
    await loaded(page,'/plates');await expect(row(page,name)).toBeVisible();
    await open(gesture,row(page,name));
    expect((await menuButtons(page)).map(s=>s.trim()).filter(s=>s!=='キュー追加')).toEqual(['編集','複製','アーカイブ','削除']);
    await page.getByRole('dialog').getByRole('button',{name:'アーカイブ',exact:true}).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(page.getByRole('status')).toHaveText(`「${name}」をアーカイブしました`);
    await expect(row(page,name)).toHaveCount(0);await expect(row(page,ctx.other.name)).toBeVisible();
    await expect(page.locator('.plate-row').first()).toBeFocused();
    await page.getByRole('link',{name:'アーカイブ済み',exact:true}).click();await expect(page).toHaveURL(/\/plates\?archived=1$/);
    await expect(page.getByRole('heading',{level:1})).toHaveText('アーカイブ');
    await expect(row(page,name)).toBeVisible();await expect(row(page,ctx.other.name)).toHaveCount(0);
    if(gesture==='right')await page.screenshot({path:`${out}/archived-list.png`});
    await open(gesture,row(page,name));
    expect((await menuButtons(page)).map(s=>s.trim())).toEqual(['復元','削除']);
    await page.getByRole('dialog').getByRole('button',{name:'復元',exact:true}).click();
    await expect(page.getByRole('status')).toHaveText(`「${name}」を復元しました`);
    await expect(page.getByText('アーカイブ済みのプレートはありません')).toBeVisible();
    if(gesture==='right')await page.screenshot({path:`${out}/archived-empty.png`});
    await page.getByRole('link',{name:'プレート一覧へ戻る',exact:true}).click();await expect(page).toHaveURL(/\/plates$/);
    await expect(row(page,name)).toBeVisible();
    // Row click still opens the detail page.
    if(gesture==='key'){await row(page,name).click();await expect(page).toHaveURL(new RegExp(`/plates/${ctx.plate.id}$`));}
  }
  expect(await (await request.get(`/api/plates/${ctx.plate.id}`)).json()).toEqual(ctx.plate);
  await context.close();
});

test('a failed archive stays in the menu, can be retried, and search works in the archived list',async({page,request})=>{
  let fail=true;
  await page.route('**/api/plates/*/archive',async route=>{if(fail){fail=false;return route.fulfill({status:503,json:{error:'unavailable'}});}return route.continue();});
  await page.setViewportSize({width:375,height:812});await loaded(page,'/plates');
  await row(page,ctx.other.name).click({button:'right'});const dialog=page.getByRole('dialog');
  const archive=dialog.getByRole('button',{name:'アーカイブ',exact:true});await archive.click();
  await expect(dialog.getByRole('alert').filter({hasText:'保存先'})).toBeVisible();await expect(archive).toBeFocused();await expect(row(page,ctx.other.name)).toBeVisible();
  await page.screenshot({path:`${out}/archive-failure.png`});
  await archive.click();await expect(dialog).toHaveCount(0);await expect(row(page,ctx.other.name)).toHaveCount(0);
  await loaded(page,'/plates?archived=1');const search=page.getByLabel('名前・モデル名で検索');
  await search.fill('洗浄');await expect(row(page,ctx.other.name)).toBeVisible();
  await search.fill('zzzz');await expect(page.getByText('一致するプレートがありません')).toBeVisible();
  await search.fill('');await row(page,ctx.other.name).click({button:'right'});await dialog.getByRole('button',{name:'復元',exact:true}).click();
  await expect(dialog).toHaveCount(0);
  expect((await (await request.get('/api/plates?archived=true')).json())).toEqual([]);
});

for(const colorScheme of ['dark','light'] as const)for(const width of [320,375,900])test(`${colorScheme} ${width}: heading keeps search and rows in place, menus return focus`,async({page,request})=>{
  await page.emulateMedia({colorScheme});await page.setViewportSize({width,height:800});await loaded(page,'/plates');
  const input=(await page.getByLabel('名前・モデル名で検索').boundingBox())!,link=(await page.getByRole('link',{name:'アーカイブ済み'}).boundingBox())!;
  const h1=(await page.getByRole('heading',{level:1}).boundingBox())!,create=(await page.getByRole('link',{name:'新規作成'}).boundingBox())!;
  expect(link.width).toBeGreaterThanOrEqual(44);
  // The icon shares the search row; the heading stays one line and nothing is pushed down.
  expect(Math.abs((link.y+link.height/2)-(input.y+input.height/2))).toBeLessThan(2);
  expect(Math.abs((create.y+create.height/2)-(h1.y+h1.height/2))).toBeLessThan(6);
  // Same position as before the change (measured on v0.1.64 with the same viewport).
  expect(Math.abs(input.y-(width>=768?152:144))).toBeLessThan(0.5);
  expect(await page.getByRole('link',{name:'アーカイブ済み'}).getAttribute('title')).toBe('アーカイブ済み');
  await fit(page);await page.screenshot({path:`${out}/list-${width}-${colorScheme}.png`});
  const target=row(page,ctx.plate.name);await target.click({button:'right'});await fit(page);
  await page.screenshot({path:`${out}/menu-${width}-${colorScheme}.png`});
  await page.keyboard.press('Escape');await expect(target).toBeFocused();
  await target.click({button:'right'});await page.getByRole('dialog').getByRole('button',{name:'閉じる',exact:true}).click();await expect(target).toBeFocused();
  expect((await request.put(`/api/plates/${ctx.plate.id}/archive`)).status()).toBe(204);
  await loaded(page,'/plates?archived=1');await fit(page);
  const back=page.getByRole('link',{name:'プレート一覧へ戻る'});expect(await back.getAttribute('title')).toBe('プレート一覧へ戻る');
  await row(page,ctx.plate.name).click({button:'right'});await page.screenshot({path:`${out}/archived-menu-${width}-${colorScheme}.png`});
  await page.keyboard.press('Escape');await expect(row(page,ctx.plate.name)).toBeFocused();
  expect((await request.delete(`/api/plates/${ctx.plate.id}/archive`)).status()).toBe(204);
});

test('200 percent text stays inside 320px in both lists and menus',async({page,request})=>{
  await page.setViewportSize({width:320,height:800});await loaded(page,'/plates');
  const before=await page.getByRole('heading',{level:1}).evaluate(e=>parseFloat(getComputedStyle(e).fontSize));
  const enlarge=()=>page.addStyleTag({content:':root {--fs-xs:24px;--fs-sm:28px;--fs-md:30px;--fs-lg:32px;--fs-xl:34px}'});
  await enlarge();expect(await page.getByRole('heading',{level:1}).evaluate(e=>parseFloat(getComputedStyle(e).fontSize))).toBe(before*2);
  await fit(page);await page.screenshot({path:`${out}/list-text200.png`,fullPage:true});
  await row(page,ctx.plate.name).click({button:'right'});await fit(page);await page.screenshot({path:`${out}/menu-text200.png`});
  // The dialog scrolls, so the last actions stay reachable at 200 percent text.
  for(const name of ['アーカイブ','削除']){const button=page.getByRole('dialog').getByRole('button',{name,exact:true});await button.scrollIntoViewIfNeeded();await expect(button).toBeInViewport();}
  await page.keyboard.press('Escape');
  expect((await request.put(`/api/plates/${ctx.plate.id}/archive`)).status()).toBe(204);
  await loaded(page,'/plates?archived=1');await enlarge();await fit(page);
  await row(page,ctx.plate.name).click({button:'right'});await fit(page);await page.screenshot({path:`${out}/archived-text200.png`});
  expect((await request.delete(`/api/plates/${ctx.plate.id}/archive`)).status()).toBe(204);
});
