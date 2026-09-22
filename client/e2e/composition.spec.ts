const emptyDefaults={"default_printer_id":null,"conditions":{"required_machine_profile_key":null,"filament_id":null,"process_profile_key":null,"bed_type":null},"reason":"printer"};
import { test, expect } from '@playwright/test';

test('a plate saves model references and quantities with nullable print conditions', async ({ page }) => {
  let saved: any;
  const plate = { id: '11111111-1111-4111-8111-111111111111', version: 1, name: '机の箱', models: [{ id: 'item-reference', name: 'parts/box.stl', source: 'parts/box.stl', quantity: 3 }] };
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname;
    if(path==='/api/default-settings')return route.fulfill({json:emptyDefaults});
    if (path === '/api/scad/models') return route.fulfill({ json: ['parts/box.stl', 'parts/lid.stl'] });
    if (path === '/api/plates/import') { saved = route.request().postDataJSON(); return route.fulfill({ status: 201, json: plate }); }
    if (path === '/api/plates/11111111-1111-4111-8111-111111111111') return route.fulfill({ json: plate });
    return route.fulfill({ json: [] });
  });
  await page.goto('/plates/new');
  await page.getByLabel('parts/box.stl', { exact: true }).check();
  await page.getByRole('button', { name: /構成を確認/ }).click();
  await page.getByLabel('プレート名', { exact: true }).fill('机の箱');
  await page.getByLabel('parts/box.stl の個数').fill('3');
  await page.getByRole('button', { name: '保存', exact: true }).click();
  await expect(page).toHaveURL(/plates\/11111111-1111-4111-8111-111111111111$/);
  await expect(page.getByRole('button', { name: '印刷キューへ' })).toBeVisible();
  expect(saved).toEqual({ name: '机の箱', conditions: {required_machine_profile_key:null,filament_id:null,process_profile_key:null,bed_type:null,sparse_infill_pattern:null,sparse_infill_density:null,wall_loops:null,brim_enabled:false}, models: [{ name: 'parts/box.stl', source: 'parts/box.stl', quantity: 3 }] });
  await expect(page.getByRole('button', { name: /parts\/box.stl.*3個/ })).toBeVisible();
  await expect(page.getByText('3個', { exact: true })).toBeVisible();
});
