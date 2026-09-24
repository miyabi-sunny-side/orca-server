import { test, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { queueFixture } from './compact-fixture';
for(const width of [375,900])for(const colorScheme of ['dark','light'] as const) {
  test(`${width} ${colorScheme}: two lines, density and stable detail/keyboard state`,async({page})=>{
    await page.setViewportSize({width,height:812});await page.emulateMedia({colorScheme});
    const {q,commands}=await queueFixture(page);await page.goto('/');
    await expect(page.locator('.job-summary')).toHaveCount(9);
    await expect(page.getByRole('link',{name:'OrcaServer',exact:true})).toHaveAttribute('aria-current','page');
    await expect(page.getByRole('link',{name:'プレート',exact:true})).toHaveAttribute('href','/plates');
    await expect(page.getByRole('button',{name:'前へ',exact:true})).toHaveCount(0);
    await expect(page.locator('.job-details:visible')).toHaveCount(0);
    const rows=page.locator('.waiting-job');
    const metrics=await rows.evaluateAll(nodes=>({visible:nodes.filter(n=>n.getBoundingClientRect().bottom<=innerHeight).length,rows:nodes.map(n=>{const lines=n.querySelector('.job-lines')!;return {height:n.getBoundingClientRect().height,children:lines.children.length,lines:[...lines.children].map(c=>({height:c.getBoundingClientRect().height,lineHeight:parseFloat(getComputedStyle(c).lineHeight),nowrap:getComputedStyle(c).whiteSpace}))};})}));
    expect(metrics.visible).toBeGreaterThanOrEqual(6);
    for(const row of metrics.rows){expect(row.children).toBe(2);for(const line of row.lines){expect(line.nowrap).toBe('nowrap');expect(line.height).toBeLessThanOrEqual(line.lineHeight+1);}}
    if(process.env.E2E_EVIDENCE_DIR){await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/queue-${width}-${colorScheme}.png`});writeFileSync(`${process.env.E2E_EVIDENCE_DIR}/queue-${width}-${colorScheme}.json`,JSON.stringify(metrics,null,2));}
    const first=rows.first();await first.locator('summary').click();
    await expect(first.getByText('PETG-GF 黒',{exact:false})).toBeVisible();
    const handle=first.getByRole('button',{name:/並べ替え/});await handle.focus();
    await page.waitForResponse(r=>r.url().includes('/api/queue')&&r.request().method()==='GET');
    await expect(handle).toBeFocused();await expect(first.locator('details')).toHaveAttribute('open','');
    const id=q.waiting[0].id;
    await handle.press('Space');await handle.press('ArrowDown');await handle.press('Enter');
    await expect.poll(()=>commands.length).toBe(1);expect(commands[0].action).toEqual({type:'move',job_id:id,index:1});
    await expect(rows.nth(1).locator('details')).toHaveAttribute('open','');
    await expect(page.locator(`#handle-${id}`)).toBeFocused();
    await page.addStyleTag({content:':root { --fs-xs:24px; --fs-sm:28px; --fs-md:30px; --fs-lg:32px; --fs-xl:34px; }'});
    await rows.nth(1).getByRole('button',{name:/キューから削除/}).scrollIntoViewIfNeeded();
    await expect(rows.nth(1).getByRole('button',{name:/キューから削除/})).toBeInViewport();
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    if(process.env.E2E_EVIDENCE_DIR)await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/queue-${width}-${colorScheme}-200.png`});
  });
}

test('pointer insertion survives polling; changed generation cancels a stale drag',async({page})=>{
  await page.setViewportSize({width:900,height:900});const {q,commands}=await queueFixture(page);await page.goto('/queue?printer_id=p1');
  const rows=page.locator('.waiting-job');await expect(rows).toHaveCount(8);
  const handle=await rows.first().getByRole('button',{name:/並べ替え/}).boundingBox();const target=await rows.nth(2).boundingBox();
  const id=q.waiting[0].id;
  await page.mouse.move(handle!.x+20,handle!.y+20);await page.mouse.down();await page.mouse.move(target!.x+22,target!.y+target!.height-8,{steps:10});
  await expect(rows.nth(2)).toHaveClass(/insert-after/);
  await page.waitForResponse(r=>r.url().includes('/api/queue')&&r.request().method()==='GET');
  await expect(rows.nth(2)).toHaveClass(/insert-after/);await page.mouse.up();
  await expect.poll(()=>commands.length).toBe(1);expect(commands[0].action).toEqual({type:'move',job_id:id,index:2});
  await expect(rows.nth(2)).toHaveAttribute('data-job-id',id);
  const next=await rows.first().getByRole('button',{name:/並べ替え/}).boundingBox();
  await page.mouse.move(next!.x+20,next!.y+20);await page.mouse.down();await page.mouse.move(next!.x+20,next!.y+90,{steps:5});
  q.generation++;
  await expect(page.locator('.dragging')).toHaveCount(0);await page.mouse.up();expect(commands).toHaveLength(1);
  await expect(page.locator('.current-job .drag-handle')).toHaveCount(0);
});

test('touch scroll starts on the card, drag is confined to the handle',async({browser})=>{
  const context=await browser.newContext({viewport:{width:375,height:520},hasTouch:true,isMobile:true});const page=await context.newPage();
  const {q,commands}=await queueFixture(page);await page.goto('/');await expect(page.locator('.waiting-job')).toHaveCount(8);
  const cdp=await context.newCDPSession(page);
  await cdp.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:240,y:450}]});
  for(let y=420;y>=130;y-=30)await cdp.send('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[{x:240,y}]});
  await cdp.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
  await expect.poll(()=>page.evaluate(()=>scrollY)).toBeGreaterThan(60);
  await expect(page.locator('.dragging')).toHaveCount(0);expect(commands).toHaveLength(0);
  await page.evaluate(()=>scrollTo(0,0));
  // Wait for native touch scrolling to settle before measuring the next gesture.
  await page.locator('.drag-handle').first().scrollIntoViewIfNeeded();
  const handle=(await page.locator('.drag-handle').first().boundingBox())!,target=(await page.locator('.waiting-job').nth(2).boundingBox())!,id=q.waiting[0].id;
  await cdp.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:handle.x+20,y:handle.y+20}]});
  await cdp.send('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[{x:handle.x+20,y:target.y+target.height-6}]});
  await expect(page.locator('.waiting-job').nth(2)).toHaveClass(/insert-after/);
  await cdp.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
  await expect.poll(()=>commands.length).toBe(1);expect(commands[0].action).toEqual({type:'move',job_id:id,index:2});
  await context.close();
});

test('home shows each printer and keeps legacy printer filtering',async({page})=>{
  const {q}=await queueFixture(page);
  await page.route('**/api/printers',r=>r.fulfill({json:[{id:'p1',name:'P1S',machine_profile_key:'Bambu Lab P1S 0.4 nozzle'},{id:'p2',name:'Second P1S',machine_profile_key:'Bambu Lab P1S 0.4 nozzle'}]}));
  await page.route('**/api/queue?printer_id=p2',r=>r.fulfill({json:{...q,current:null,waiting:[]}}));
  await page.route('**/api/printers/p2/ams',r=>r.fulfill({json:{slots:[]}}));
  await page.goto('/');await expect(page.locator('.printer-queue')).toHaveCount(2);
  await page.getByLabel('表示するプリンター').selectOption('p2');await expect(page.locator('.printer-queue')).toHaveCount(1);await expect(page.getByRole('heading',{name:'Second P1S',exact:true})).toBeVisible();
  await page.goto('/queue?printer_id=p1');await expect(page.locator('.job-summary')).toHaveCount(9);await expect(page.locator('.printer-queue')).toHaveCount(1);
});
