const emptyDefaults={"default_printer_id":null,"conditions":{"required_machine_profile_key":null,"filament_id":null,"process_profile_key":null,"bed_type":null},"reason":"printer"};
import { test, expect } from '@playwright/test';
import { readFileSync, mkdirSync } from 'node:fs';
const cube = readFileSync('../tests/fixtures/cube.stl');
const triangle = readFileSync('../tests/fixtures/triangle.stl');
const id = '11111111-1111-4111-8111-111111111111';
const plate = { id, version: 1, name: '形状を見て選ぶプレート', models: [
  { id:'cube', name:'立方体.stl', source:'parts/cube.stl', quantity:10 },
  { id:'triangle', name:'三角形.stl', source:null, quantity:1 },
] };
const output=process.env.E2E_EVIDENCE_DIR;

test('real WebGL distinguishes models, ignores stale reads and survives errors and remounts',async({page})=>{
  let mode='normal',slow=false,release:()=>void=()=>{};
  const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));
  await page.route('**/api/**',async route=>{
    const path=new URL(route.request().url()).pathname;
    if(path==='/api/default-settings')return route.fulfill({json:emptyDefaults});
    if(path===`/api/plates/${id}`)return route.fulfill({json:plate});
    if(path.includes('/models/')){
      if(mode==='missing')return route.fulfill({status:404});
      if(mode==='broken')return route.fulfill({body:'not an STL'});
      if(mode==='network')return route.abort();
      const isTriangle=path.endsWith('/triangle');
      if(isTriangle&&mode==='slow'){slow=true;await new Promise<void>(r=>release=r);}
      return route.fulfill({contentType:'application/octet-stream',body:isTriangle?triangle:cube});
    }
    return route.fulfill({json:[]});
  });
  await page.goto(`/plates/${id}`);
  const preview=page.getByRole('complementary',{name:'STLプレビュー'});
  const chooseCube=page.getByRole('button',{name:/立方体.stl/});
  const chooseTriangle=page.getByRole('button',{name:/三角形.stl/});
  await expect(preview.getByRole('status')).toHaveText('20.0 × 20.0 × 20.0 mm');
  await expect(preview.locator('canvas')).toHaveCount(1);
  for(const scheme of ['dark','light'] as const)for(const width of [320,900]){
    await page.emulateMedia({colorScheme:scheme});await page.setViewportSize({width,height:900});
    await preview.getByRole('button',{name:'全体を表示',exact:true}).click();
    const controls=await page.locator('.controls').boundingBox(),view=await preview.boundingBox();
    expect(controls&&view).toBeTruthy();
    if(width===900)expect(view!.x).toBeGreaterThanOrEqual(controls!.x+controls!.width);
    else expect(view!.y).toBeGreaterThanOrEqual(controls!.y+controls!.height);
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    if(output){mkdirSync(output,{recursive:true});await page.evaluate(()=>window.scrollTo(0,0));await page.screenshot({path:`${output}/preview-${scheme}-${width}.png`,fullPage:true});}
  }
  await page.setViewportSize({width:900,height:900});
  await chooseTriangle.click();await expect(preview.getByRole('status')).toHaveText('1.0 × 1.0 × 0.0 mm');
  await expect(chooseTriangle).toHaveAttribute('aria-pressed','true');
  if(output)await preview.screenshot({path:`${output}/triangle.png`});
  await chooseCube.click();await expect(preview.getByRole('status')).toContainText('20.0');
  mode='slow';await chooseTriangle.click();await expect.poll(()=>slow).toBe(true);
  const aborted=page.waitForEvent('requestfailed',r=>r.url().endsWith('/models/triangle'));
  await chooseCube.click();await aborted;await expect(preview.getByRole('status')).toContainText('20.0');
  release();
  await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));
  await expect(preview.getByRole('status')).toHaveText('20.0 × 20.0 × 20.0 mm');
  for(mode of ['missing','broken','network']){
    await preview.getByRole('button',{name:'読み直す',exact:true}).click();
    await expect(preview.getByRole('alert')).toContainText('STLを表示できません');
    await expect(page.getByRole('button',{name:'構成を編集',exact:true})).toBeEnabled();
  }
  mode='normal';await preview.getByRole('button',{name:'読み直す',exact:true}).click();
  await expect(preview.getByRole('status')).toContainText('20.0');
  const before=await preview.locator('canvas').screenshot();
  await preview.getByRole('button',{name:'拡大',exact:true}).click();
  await expect.poll(async()=>before.equals(await preview.locator('canvas').screenshot())).toBe(false);
  await preview.getByRole('application').focus();await page.keyboard.press('Shift+ArrowRight');
  await page.getByRole('button',{name:'構成を編集',exact:true}).click();await page.mouse.move(0,0);await expect(page.locator('canvas')).toHaveCount(0);
  await page.getByRole('button',{name:'編集をやめる',exact:true}).click();await expect(preview.getByRole('status')).toContainText('20.0');
  await expect(page.locator('canvas')).toHaveCount(1);
  await page.goto('/');await expect(page.locator('canvas')).toHaveCount(0);
  await page.goto(`/plates/${id}`);await expect(preview.getByRole('status')).toContainText('20.0');
  await page.addStyleTag({content:'html {font-size:200% !important}'});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  expect(errors).toEqual([]);
});

test('WebGL unavailable keeps composition editing usable',async({page})=>{
  await page.addInitScript(()=>{
    const original=HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext=function(type:any,...args:any[]){
      return String(type).startsWith('webgl')?null:(original as any).call(this,type,...args);
    } as any;
  });
  await page.route('**/api/**',route=>route.fulfill({json:new URL(route.request().url()).pathname==='/api/default-settings'?emptyDefaults:new URL(route.request().url()).pathname===`/api/plates/${id}`?plate:[]}));
  await page.goto(`/plates/${id}`);
  await expect(page.getByRole('alert')).toContainText('WebGL');
  await page.getByRole('button',{name:'構成を編集',exact:true}).click();
  await expect(page.getByLabel('プレート名',{exact:true})).toHaveValue(plate.name);
});

test('sticky search retains selected models across scrolling, filtering and saving',async({page})=>{
  const names=Array.from({length:100},(_,i)=>`parts/item-${String(i).padStart(3,'0')}.stl`);let saved:any;
  await page.setViewportSize({width:320,height:812});
  await page.route('**/api/**',route=>{
    const url=new URL(route.request().url());
    if(url.pathname==='/api/default-settings')return route.fulfill({json:emptyDefaults});
    if(url.pathname==='/api/scad/models')return route.fulfill({json:names.filter(n=>n.includes(url.searchParams.get('q')??''))});
    if(url.pathname==='/api/plates/import'){saved=route.request().postDataJSON();return route.fulfill({status:201,json:{...plate,...saved}});}
    if(url.pathname===`/api/plates/${id}`)return route.fulfill({json:{...plate,...saved}});
    return route.fulfill({json:[]});
  });
  await page.goto('/plates/new');await page.getByLabel(names[0],{exact:true}).check();
  await page.evaluate(()=>window.scrollTo(0,3000));
  const search=page.getByRole('searchbox',{name:'モデル名で検索'});
  const box=await search.boundingBox();expect(box!.y).toBeGreaterThanOrEqual(48);expect(box!.y+box!.height).toBeLessThan(812);
  await expect(page.getByRole('button',{name:'構成を確認（1）',exact:true})).toBeInViewport();
  if(output)await page.screenshot({path:`${output}/sticky-search.png`});
  await search.fill('099');await page.getByLabel(names[99],{exact:true}).check();
  await page.getByRole('button',{name:'構成を確認（2）',exact:true}).click();
  await page.getByLabel('プレート名',{exact:true}).fill('選択を保持');await page.getByLabel(names[0]+' の個数').fill('10');
  await page.getByRole('button',{name:'モデル選択へ',exact:true}).click();
  await expect(search).toHaveValue('099');await expect(page.getByLabel(names[99],{exact:true})).toBeChecked();
  await page.getByRole('button',{name:'構成を確認（2）',exact:true}).click();
  await expect(page.getByLabel(names[0]+' の個数')).toHaveValue('10');
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(`/plates/${id}`);
  expect(saved.models.map((m:any)=>[m.source,m.quantity])).toEqual([[names[0],10],[names[99],1]]);
});
