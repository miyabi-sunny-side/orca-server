import { test, expect, type Page } from '@playwright/test';
const id='11111111-1111-4111-8111-111111111111';
async function menu(page:Page) {
  await page.route('**/api/**',route=>{
    const path=new URL(route.request().url()).pathname;
    return route.fulfill({json:path==='/api/plates'?[{id,version:1,name:'天馬ルームケース: base前',models:[{id:'item',name:'bin.stl',source:'bin.stl',quantity:10}],conditions:{}}]:[]});
  });
  await page.goto('/plates');await page.locator('.plate-row').press('Shift+F10');
  await page.getByRole('dialog').getByRole('button',{name:'複製',exact:true}).click();
}
test('cancel paths restore focus without creating; failed and repeated submits retain the name',async({page})=>{
  await menu(page);const dialog=page.getByRole('dialog'),name=dialog.getByLabel('プレート名');
  let count=0, release!:()=>void;const hold=new Promise<void>(r=>release=r);
  await page.route('**/duplicate',async route=>{count++;await hold;await route.fulfill({status:503,json:{error:'Storage unavailable'}});});
  await expect(name).toBeFocused();await expect(name).toHaveValue('天馬ルームケース: base前');
  for(const cancel of ['キャンセル','Escape','閉じる']) {
    if(cancel==='Escape')await page.keyboard.press('Escape');else await dialog.getByRole('button',{name:cancel,exact:true}).click();
    await expect(dialog.getByRole('button',{name:'複製',exact:true})).toBeFocused();
    expect(count).toBe(0);await page.keyboard.press('Enter');await expect(name).toBeFocused();
  }
  await name.fill('');await dialog.getByRole('button',{name:'複製',exact:true}).click();expect(count).toBe(0);await expect(name).toBeFocused();
  await name.fill('天馬ルームケース: base後');
  await dialog.getByRole('button',{name:'複製',exact:true}).evaluate((e:HTMLButtonElement)=>{e.click();e.click();});
  await expect.poll(()=>count).toBe(1);await expect(name).toBeDisabled();await expect(dialog.getByRole('button',{name:'閉じる'})).toBeDisabled();
  await page.keyboard.press('Escape');await expect(dialog).toBeVisible();await page.keyboard.press('Tab');await expect(dialog).toBeFocused();
  release();await expect(dialog.getByRole('alert')).toContainText('保存先');await expect(name).toHaveValue('天馬ルームケース: base後');await expect(name).toBeFocused();
  await page.unroute('**/duplicate');await page.route('**/duplicate',route=>route.fulfill({status:400,json:{error:'Name must contain 1–256 bytes without control characters'}}));
  await name.fill('あ'.repeat(86));await dialog.getByRole('button',{name:'複製',exact:true}).click();await expect(dialog.getByRole('alert')).toContainText('プレート名');await expect(name).toHaveValue('あ'.repeat(86));
});
