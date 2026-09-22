import { test, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';
const ctx=JSON.parse(process.env.E2E_SUPPORT_CONTEXT ?? '{}');
const out=process.env.E2E_EVIDENCE_DIR!;

test('support selection stays optional, searchable and persistent in both themes',async({page,request})=>{
  test.setTimeout(120_000);mkdirSync(out,{recursive:true});
  for(const [width,colorScheme] of [[375,'dark'],[375,'light'],[900,'dark'],[900,'light']] as const){
    await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme});
    await page.goto('/plates/new');await page.getByRole('checkbox').check();await page.getByRole('button',{name:'構成を確認（1）'}).click();
    await page.getByLabel('プレート名',{exact:true}).fill(`接触面の材料 ${width} ${colorScheme}`);
    const details=page.locator('details').filter({has:page.locator('summary',{hasText:'詳細設定'})});
    await expect(details).not.toHaveAttribute('open');
    await expect(page.getByRole('button',{name:/接触面のフィラメント/})).toHaveCount(0);
    await details.locator('summary').focus();await page.keyboard.press('Enter');
    const support=page.getByLabel('サポートを使う');await expect(support).not.toBeChecked();
    await support.check();
    const selector=page.getByRole('button',{name:/接触面のフィラメント/});
    await expect(selector).toContainText('PLA 白');
    await selector.click();const search=page.getByRole('searchbox',{name:'材料を検索'});await expect(search).toBeFocused();
    await search.fill('PLA');await expect(page.locator('.choices button')).toHaveCount(2);
    await page.getByRole('button',{name:/PLA 青.*Fixture/}).click();await expect(selector).toContainText('PLA 青');await expect(selector).toBeFocused();
    await support.uncheck();await expect(selector).toHaveCount(0);await support.check();await expect(selector).toContainText('PLA 青');
    await selector.click();await page.getByLabel('所持していないフィラメントを選択する').check();await search.fill('PETG-GF');await expect(page.locator('.choices button')).toHaveCount(1);
    await search.press('ArrowDown');await expect(page.locator('.choices button')).toBeFocused();await page.keyboard.press('Enter');
    await expect(selector).toContainText('PETG-GF 黒');
    await selector.click();await search.fill('見つからない材料');await expect(page.getByText('一致する材料がありません。')).toBeVisible();
    await search.press('Escape');await expect(selector).toContainText('PETG-GF 黒');await expect(selector).toBeFocused();
    expect(await support.evaluate(e=>e.parentElement!.getBoundingClientRect().height)).toBeGreaterThanOrEqual(44);
    expect(await selector.evaluate(e=>e.getBoundingClientRect().height)).toBeGreaterThanOrEqual(44);
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`${out}/selection-${width}-${colorScheme}.png`,fullPage:true});
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);
    const id=new URL(page.url()).pathname.split('/')[2];
    expect((await (await request.get('/api/plates/'+id)).json()).conditions).toMatchObject({support_enabled:true,support_interface_filament_id:ctx.gf});
    await expect(page.getByText('接触面用のフィラメントをAMSに装填し、材料を割り当ててください。')).toBeVisible();
    await expect(page.getByRole('link',{name:'AMSを確認'})).toHaveAttribute('href','/printers/p1/ams');
    await expect(page.getByRole('button',{name:'印刷キューへ',exact:true})).toBeDisabled();
    await page.screenshot({path:`${out}/missing-${width}-${colorScheme}.png`,fullPage:true});
    await page.reload();await page.getByRole('button',{name:'構成を編集'}).click();await details.locator('summary').click();
    await expect(support).toBeChecked();await expect(selector).toContainText('PETG-GF 黒');
    await selector.click();await search.fill('PLA 青');await page.getByRole('button',{name:/PLA 青.*Fixture/}).click();
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('button',{name:'構成を編集'})).toBeVisible();
    await page.getByRole('button',{name:'印刷キューへ',exact:true}).click();await expect(page.getByRole('link',{name:'キューを見る'})).toBeVisible();
    await expect(page.getByRole('searchbox',{name:'材料を検索'})).toHaveCount(0);
    const q=await (await request.get('/api/queue?printer_id=p1')).json();const job=q.waiting.find((j:any)=>j.plate_id===id);expect(job).toBeTruthy();
    expect((await request.post('/api/queue?printer_id=p1',{data:{epoch:q.epoch,generation:q.generation,request_id:q.request_id,action:{type:'remove',job_id:job.id}}})).ok()).toBe(true);
  }
});
