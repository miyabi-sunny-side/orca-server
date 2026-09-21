import { test, expect } from '@playwright/test';
import { readFileSync, mkdirSync } from 'node:fs';
const cube=readFileSync('../tests/fixtures/cube.stl'),triangle=readFileSync('../tests/fixtures/triangle.stl');
const longBox=Buffer.from(cube);
for(let i=0;i<longBox.readUInt32LE(80);i++)for(let j=0;j<3;j++){
  const offset=84+i*50+12+j*12;longBox.writeFloatLE(longBox.readFloatLE(offset)*5,offset);longBox.writeFloatLE(longBox.readFloatLE(offset+8)/4,offset+8);
}
const names=['parts/cube.stl','parts/triangle.stl','長いモデル名/'.repeat(8)+'横長の形状.stl',...Array.from({length:60},(_,i)=>`parts/extra-${i}.stl`)];
const output=process.env.E2E_EVIDENCE_DIR;

test('hover and row focus show one passive shape without changing composition or covering controls',async({page})=>{
  let mode='normal',reads=0,slow=false,release=()=>{};
  const mutations:string[]=[],errors:string[]=[];
  page.on('pageerror',e=>errors.push(e.message));
  await page.route('**/api/**',async route=>{
    const r=route.request(),url=new URL(r.url());
    if(r.method()!=='GET')mutations.push(r.method()+' '+url.pathname);
    if(url.pathname==='/api/scad/models')return route.fulfill({json:names});
    if(url.pathname==='/api/scad/model'){
      reads++;const isTriangle=url.searchParams.get('path')===names[1];
      if(mode==='slow'&&isTriangle){slow=true;await new Promise<void>(r=>release=r);}
      return route.fulfill({status:mode==='error'?502:200,contentType:'application/octet-stream',body:isTriangle?triangle:url.searchParams.get('path')===names[2]?longBox:cube});
    }
    return route.fulfill({json:[]});
  });
  await page.emulateMedia({colorScheme:'dark'});await page.setViewportSize({width:900,height:900});await page.goto('/plates/new');
  await expect(page.getByLabel(names[0],{exact:true})).toBeVisible();expect(reads).toBe(0);
  const name=(i:number)=>page.locator('[data-stl-preview]').filter({hasText:names[i]});
  const popup=page.getByRole('tooltip'),canvas=popup.locator('canvas');
  async function visible(i:number,dimensions:string){
    await name(i).hover();await expect(popup.getByRole('status')).toHaveText(dimensions);
    await expect(canvas).toHaveCount(1);await expect(popup.getByRole('button')).toHaveCount(0);
    await expect(canvas).toHaveAttribute('role','img');
    const clear=await popup.evaluate(el=>{
      const p=el.getBoundingClientRect();
      return p.left>=0&&p.top>=0&&p.right<=innerWidth&&p.bottom<=innerHeight&&
        [...document.querySelectorAll('input,button,select,textarea,header')].every(c=>{
          const r=c.getBoundingClientRect();return !r.width||!r.height||p.right<=r.left||p.left>=r.right||p.bottom<=r.top||p.top>=r.bottom;
        });
    });expect(clear).toBe(true);
  }
  await visible(0,'20.0 × 20.0 × 20.0 mm');const first=await canvas.screenshot();
  if(output){mkdirSync(output,{recursive:true});await page.screenshot({path:`${output}/hover-cube-dark.png`});}
  await visible(1,'1.0 × 1.0 × 0.0 mm');expect(first.equals(await canvas.screenshot())).toBe(false);
  if(output)await page.screenshot({path:`${output}/hover-triangle-dark.png`});
  await canvas.evaluate(el=>el.addEventListener('webglcontextlost',()=>{(window as any).hoverContextLost=true;}));
  await page.mouse.move(0,899);await expect(popup).toHaveCount(0);await expect(page.locator('canvas')).toHaveCount(0);
  await expect.poll(()=>page.evaluate(()=>(window as any).hoverContextLost)).toBe(true);
  await page.emulateMedia({colorScheme:'light'});await visible(2,'100.0 × 20.0 × 5.0 mm');
  if(output)await page.screenshot({path:`${output}/hover-long-light.png`});
  await page.mouse.wheel(0,500);await expect.poll(()=>page.evaluate(()=>scrollY)).toBeGreaterThan(0);await expect(popup).toHaveCount(0);
  await page.evaluate(()=>scrollTo(0,0));
  mode='slow';await name(1).hover();await expect.poll(()=>slow).toBe(true);
  const aborted=page.waitForEvent('requestfailed',r=>new URL(r.url()).searchParams.get('path')===names[1]);
  await visible(0,'20.0 × 20.0 × 20.0 mm');await aborted;release();
  await expect(popup.getByRole('status')).toHaveText('20.0 × 20.0 × 20.0 mm');
  slow=false;await name(1).hover();await expect.poll(()=>slow).toBe(true);
  const closed=page.waitForEvent('requestfailed',r=>new URL(r.url()).searchParams.get('path')===names[1]);
  await page.mouse.move(0,899);await closed;release();await expect(page.locator('canvas')).toHaveCount(0);
  mode='error';await name(1).hover();await expect(popup.getByRole('alert')).toBeVisible();
  await page.getByLabel(names[0],{exact:true}).check();await expect(page.getByLabel(names[0],{exact:true})).toBeChecked();
  mode='normal';await page.getByLabel(names[0],{exact:true}).focus();await page.keyboard.press('ArrowDown');
  await expect(page.getByLabel(names[1],{exact:true})).toBeFocused();await expect(popup.getByRole('status')).toContainText('1.0');
  await page.keyboard.press('Escape');await expect(popup).toHaveCount(0);
  await page.getByRole('button',{name:'構成を確認（1）',exact:true}).click();
  const quantity=page.getByLabel(`${names[0]} の個数`);await quantity.fill('10');
  await visible(0,'20.0 × 20.0 × 20.0 mm');await expect(quantity).toHaveValue('10');
  await expect(page.getByRole('button',{name:'保存',exact:true})).toBeEnabled();
  if(output)await page.screenshot({path:`${output}/hover-composition.png`});
  await page.setViewportSize({width:320,height:812});await page.getByLabel('プレート名',{exact:true}).focus();await quantity.focus();
  await expect(popup.getByRole('status')).toHaveText('20.0 × 20.0 × 20.0 mm');
  if(output)await page.screenshot({path:`${output}/hover-keyboard-320.png`});
  expect(mutations).toEqual([]);
  await page.goto('/');await expect(page.locator('canvas')).toHaveCount(0);expect(errors).toEqual([]);
});

test('touch selection and quantity entry remain usable without a hover panel',async({browser})=>{
  const context=await browser.newContext({viewport:{width:320,height:812},hasTouch:true,isMobile:true});
  const page=await context.newPage();let reads=0;
  await page.route('**/api/**',route=>{
    const path=new URL(route.request().url()).pathname;
    if(path==='/api/scad/model')reads++;
    return route.fulfill({json:path==='/api/scad/models'?names:[]});
  });
  await page.goto('/plates/new');await page.getByText(names[0],{exact:true}).tap();
  await expect(page.getByLabel(names[0],{exact:true})).toBeChecked();
  await page.getByRole('button',{name:'構成を確認（1）',exact:true}).tap();
  const quantity=page.getByLabel(`${names[0]} の個数`);await quantity.tap();await quantity.fill('10');
  await expect(quantity).toHaveValue('10');await expect(page.getByRole('tooltip')).toHaveCount(0);expect(reads).toBe(0);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  if(output)await page.screenshot({path:`${output}/hover-touch-320.png`,fullPage:true});
  await context.close();
});

test('editing an uploaded model previews its owned bytes without a SCAD connection',async({page})=>{
  const id='11111111-1111-4111-8111-111111111111';let publicReads=0;
  const plate={id,version:1,name:'保存済みアップロード',models:[{id:'upload',name:'upload.stl',source:null,quantity:5}]};
  await page.route('**/api/**',route=>{
    const path=new URL(route.request().url()).pathname;
    if(path===`/api/plates/${id}`)return route.fulfill({json:plate});
    if(path===`/api/plates/${id}/models/upload`)return route.fulfill({contentType:'application/octet-stream',body:cube});
    if(path==='/api/scad/model')publicReads++;
    return route.fulfill({status:path.startsWith('/api/scad/')?503:200,json:[]});
  });
  await page.goto(`/plates/${id}`);await page.getByRole('button',{name:'構成を編集',exact:true}).click();
  await page.locator('[data-stl-preview]').hover();
  await expect(page.getByRole('tooltip').getByRole('status')).toHaveText('20.0 × 20.0 × 20.0 mm');
  await expect(page.getByLabel('upload.stl の個数')).toHaveValue('5');expect(publicReads).toBe(0);
  await page.keyboard.press('Escape');await expect(page.locator('canvas')).toHaveCount(0);
});
