import { expect, type Page } from '@playwright/test';
export const materialButton=(page:Page)=>page.getByRole('button',{name:/^フィラメント:/});
export async function selectMaterial(page:Page,id:string|null) {
  await materialButton(page).click();
  const modal=page.getByRole('dialog',{name:'フィラメントを選択'});
  if(id===null) await modal.getByRole('button',{name:'指定を解除',exact:true}).click();
  else {
    await modal.getByLabel('所持していないフィラメントを選択する').check();
    await modal.locator(`button[data-filament-id="${id}"]`).click();
  }
  await expect(modal).toHaveCount(0);
}
export async function expectMaterial(page:Page,id:string|null) {
  if(id===null) await expect(materialButton(page)).toContainText('フィラメントを選択');
  else {
    const list=await (await page.request.get('/api/filaments')).json();
    await expect(materialButton(page)).toContainText(list.find((f:{id:string})=>f.id===id).name);
  }
}
