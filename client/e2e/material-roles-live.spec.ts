import { test, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';
const ctx = JSON.parse(process.env.E2E_MATERIAL_ROLES_CONTEXT ?? '{}');
const out = process.env.E2E_EVIDENCE_DIR!;

test('role materials save, reopen and estimate with keyboard operation in both themes',async({page,request})=>{
  test.setTimeout(150_000); mkdirSync(out,{recursive:true});
  const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));
  for(const [width,colorScheme] of [[320,'dark'],[320,'light'],[900,'dark'],[900,'light']] as const){
    await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme});
    await page.goto('/plates/new');await page.getByLabel('parts/roles.3mf',{exact:true}).check();
    await page.getByRole('button',{name:'構成を確認（1）'}).click();
    await page.getByLabel('プレート名',{exact:true}).fill(`役割材料 ${width} ${colorScheme}`);
    const primary=page.getByRole('button',{name:/^primaryのフィラメント:/});
    const secondary=page.getByRole('button',{name:/^secondaryのフィラメント:/});
    await expect(primary).toContainText('PLA 白');
    await expect(secondary).toContainText('フィラメントを選択');
    await secondary.focus();await page.keyboard.press('Enter');
    const dialog=page.getByRole('dialog',{name:'secondaryのフィラメントを選択'});
    const search=dialog.getByRole('searchbox',{name:'材料を検索'});
    await expect(search).toBeFocused();await search.fill('PLA 青');
    await expect(dialog.locator('.choices button')).toHaveCount(1);
    await search.press('ArrowDown');await expect(dialog.locator(`button[data-filament-id="${ctx.secondary}"]`)).toBeFocused();
    await page.keyboard.press('Enter');await expect(secondary).toBeFocused();await expect(secondary).toContainText('PLA 青');
    await primary.click();await page.getByRole('searchbox',{name:'材料を検索'}).press('Escape');await expect(primary).toBeFocused();
    for(const button of [primary,secondary])expect((await button.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`${out}/roles-${width}-${colorScheme}.png`,fullPage:true});
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);
    const id=new URL(page.url()).pathname.split('/')[2];
    let saved=await (await request.get('/api/plates/'+id)).json();
    expect(saved.conditions).toMatchObject({filament_id:ctx.primary,secondary_filament_id:ctx.secondary});
    expect(saved.models[0].roles).toEqual(['primary','secondary']);
    await page.reload();await page.getByRole('button',{name:'構成を編集'}).click();
    await expect(secondary).toContainText('PLA 青');
    if(width===320&&colorScheme==='dark'){
      await secondary.click();await page.getByRole('button',{name:'指定を解除',exact:true}).click();
      await page.getByRole('button',{name:'保存',exact:true}).click();
      await expect(page.getByText('secondaryの材料を設定してください。',{exact:true})).toBeVisible();
      await expect(page.getByRole('button',{name:'印刷キューへ',exact:true})).toBeDisabled();
      await page.getByRole('button',{name:'構成を編集'}).click();await secondary.click();
      await page.locator(`button[data-filament-id="${ctx.secondary}"]`).click();
    }
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('button',{name:'構成を編集'})).toBeVisible();
    await page.getByRole('button',{name:'印刷キューへ',exact:true}).click();await expect(page.getByRole('link',{name:'キューを見る'})).toBeVisible();
    await expect.poll(async()=>{
      const queue=await (await request.get('/api/queue?printer_id=p1')).json();
      return queue.waiting.find((j:any)=>j.plate_id===id)?.estimate.state;
    },{timeout:60_000}).toBe('ready');
    const queue=await (await request.get('/api/queue?printer_id=p1')).json();
    const job=queue.waiting.find((j:any)=>j.plate_id===id);
    expect((await request.post('/api/queue?printer_id=p1',{data:{epoch:queue.epoch,generation:queue.generation,request_id:queue.request_id,action:{type:'remove',job_id:job.id}}})).ok()).toBe(true);
  }
  await page.goto('/plates/new');await page.getByLabel('parts/cube.stl',{exact:true}).check();
  await page.getByRole('button',{name:'構成を確認（1）'}).click();
  await expect(page.getByRole('button',{name:/^フィラメント:/})).toBeVisible();
  await expect(page.getByRole('button',{name:/secondaryのフィラメント/})).toHaveCount(0);
  expect(errors).toEqual([]);
});
