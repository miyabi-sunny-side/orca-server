import { test, expect } from '@playwright/test';

const machine = 'Bambu Lab P1S 0.4 nozzle';
const quality = '0.20mm Standard @BBL X1C';
const bed = 'Textured PEI Plate';
for (const theme of ['dark', 'light'] as const) {
  test(`${theme}: waiting settings retain their version while another screen changes the queue`, async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 }); await page.emulateMedia({ colorScheme: theme });
    const specification = { filament_id: 'white', ams_slot_id: 'slot', required_machine_profile_key: machine, process_profile_key: quality, bed_type: bed };
    const job = { id: 'job', plate_id: 'plate', name: '長い名前の配線整理・ケーブルホルダー', state: 'queued', ...specification, hold_reason: null, attempt_id: null, artifact_path: null, last_error: null };
    let generation = 1, sent: any, reads = 0;
    const printer = { id: 'p1', name: 'P1S', machine_profile_key: machine, default_process_profile_key: quality, bed_type: bed };
    const materials = ['white', 'blue'].map((id, i) => ({ id, name: i ? 'PLA 青' : 'PLA 白', vendor: 'Fixture', material: 'PLA', color: i ? '00FFFFFF' : 'FFFFFFFF' }));
    await page.route('**/api/**', async route => {
      const path = new URL(route.request().url()).pathname;
      if (path === '/api/printers') return route.fulfill({ json: [printer] });
      if (path === '/api/printers/profiles') return route.fulfill({ json: [{ key: machine, model: 'P1S', nozzle_diameter: '0.4' }] });
      if (path === '/api/filaments') return route.fulfill({ json: materials });
      if (path.startsWith('/api/filaments/')) return route.fulfill({ json: { settings: [{ machine_profile_key: machine }] } });
      if (path === '/api/slicer/profiles') return route.fulfill({ json: { processes: [quality], beds: [bed], defaults: { process: quality, bed } } });
      if (path === '/api/printers/p1/ams') return route.fulfill({ json: { current: true, slots: [{ id: 'slot', ams_id: 0, slot_index: 3, filament: materials[0] }] } });
      if (path === '/api/queue') {
        if (route.request().method() === 'POST') { sent = route.request().postDataJSON(); return route.fulfill({ status: 409, json: { error: 'queue changed' } }); }
        reads++;
        return route.fulfill({ json: { epoch: 'epoch', generation, request_id: `request-${reads}`, current: null, waiting: [job], allowed: { next: true, retry: false, discard: false }, printer: { ready_to_print: true, connection: 'connected', synchronized: true, print: { error: 0 } } } });
      }
      return route.fulfill({ json: [] });
    });
    await page.goto('/queue?printer_id=p1');
    await page.getByRole('button', { name: '材料・印刷条件を変更' }).click();
    await page.getByLabel('使用予定の材料').selectOption('blue');
    await expect(page.getByRole('button', { name: '待機設定を保存' })).toBeEnabled();
    generation = 2; const previousReads = reads;
    await expect.poll(() => reads).toBeGreaterThan(previousReads);
    await page.getByRole('button', { name: '待機設定を保存' }).click();
    await expect(page.getByRole('alert')).toContainText('状態が変わりました');
    expect(sent.generation).toBe(1); expect(sent.epoch).toBe('epoch');
    expect(sent.action.specification).toEqual({ ...specification, filament_id: 'blue' });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(await page.locator('.btn.primary').count()).toBe(1);
    await page.evaluate(() => scrollTo(0, 0));
    if (process.env.E2E_EVIDENCE_DIR) await page.screenshot({ path: `${process.env.E2E_EVIDENCE_DIR}/waiting-edit-${theme}.png`, fullPage: true });
    await page.getByRole('button', { name: '最新状態を読み直す' }).click();
    await expect(page.getByLabel('使用予定の材料')).toHaveCount(0);
    await page.getByRole('button', { name: '材料・印刷条件を変更' }).click();
    await expect(page.getByLabel('使用予定の材料')).toHaveValue('white');
  });
}
