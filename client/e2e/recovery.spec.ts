import { test, expect } from '@playwright/test';
import { queueFixture } from './compact-fixture';

for (const width of [320, 900]) for (const colorScheme of ['dark', 'light'] as const) {
  test(`${width} ${colorScheme}: stopped recovery stays visible and takes one keyboard action`, async ({page}) => {
    await page.setViewportSize({width, height:812}); await page.emulateMedia({colorScheme});
    page.on('dialog', () => { throw new Error('No extra confirmation dialog'); });
    const {q, commands} = await queueFixture(page);
    q.current.state = 'needs_attention'; q.printer.print.state = 'FAILED';
    q.allowed.retry = true; q.allowed.discard = true;
    Object.assign(q, {recovery:{retry_reason:null,discard_reason:null}});
    await page.goto('/');
    await page.locator('.current-job').waitFor();
    if (process.env.E2E_EVIDENCE_DIR) await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/initial-${width}-${colorScheme}.png`});
    await expect(page.locator('.job-details:visible')).toHaveCount(0);
    const retry = page.getByRole('button', {name:'取り外した・最初から再印刷',exact:true});
    await expect(retry).toBeVisible(); await expect(retry).toBeEnabled();
    await expect(page.getByRole('checkbox')).toHaveCount(0);
    if (process.env.E2E_EVIDENCE_DIR) await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/recovery-${width}-${colorScheme}.png`});
    expect((await retry.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    await retry.focus(); await retry.press('Enter');
    await expect.poll(() => commands.length).toBe(1);
    expect(commands[0].action).toEqual({type:'retry', expected_job:q.current.id, cleared:true});
    q.allowed.retry = false; q.allowed.discard = false;
    Object.assign(q, {recovery:{retry_reason:'Printer report does not match the recovery target',discard_reason:'Printer report does not match the recovery target'}});
    await page.reload();
    await expect(retry).toBeVisible(); await expect(retry).toBeDisabled();
    await expect(page.getByText('本体のジョブが変わっています。印刷状況を確認してください。',{exact:true})).toBeVisible();
    await page.addStyleTag({content:':root { --fs-xs:24px; --fs-sm:28px; --fs-md:30px; --fs-lg:32px; --fs-xl:34px; }'});
    await retry.scrollIntoViewIfNeeded(); await expect(retry).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    if (process.env.E2E_EVIDENCE_DIR) await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/recovery-blocked-${width}-${colorScheme}.png`});
    expect(commands).toHaveLength(1);
  });
}
