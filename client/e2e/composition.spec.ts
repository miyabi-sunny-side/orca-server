import { test, expect } from '@playwright/test';

test('a plate saves model references and quantities without print settings', async ({ page }) => {
  let saved: any;
  const plate = { id: '11111111-1111-4111-8111-111111111111', version: 1, name: '机の箱', models: [{ id: 'item-reference', name: 'parts/box.stl', source: 'parts/box.stl', quantity: 3 }] };
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname;
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
  await expect(page.getByRole('link', { name: '印刷キューへ' })).toBeVisible();
  expect(saved).toEqual({ name: '机の箱', models: [{ name: 'parts/box.stl', source: 'parts/box.stl', quantity: 3 }] });
  await expect(page.getByText('parts/box.stl', { exact: true })).toBeVisible();
  await expect(page.getByText('3個', { exact: true })).toBeVisible();
});
