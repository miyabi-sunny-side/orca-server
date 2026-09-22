import {test,expect} from '@playwright/test';
const machine='Bambu Lab P1S 0.4 nozzle';
async function fixture(page:any,title?:string){
  let plates=[1,2].map(i=>({id:`10000000-0000-4000-8000-${String(i).padStart(12,'0')}`,version:1,name:title??`保存プレート ${i}`,models:[{id:'model',name:'cube.stl',source:'cube.stl',quantity:1}],conditions:{required_machine_profile_key:machine,filament_id:'white',process_profile_key:'quality',bed_type:'Cool Plate'}}));
  const printers=[{id:'p1',name:'P1S',machine_profile_key:machine}];let allowed=true;const commands:any[]=[]; const deletes:string[]=[];
  await page.route('**/api/**',async(route:any)=>{
    const url=new URL(route.request().url()),path=url.pathname;
    if(path==='/api/plates')return route.fulfill({json:plates});
    if(path.startsWith('/api/plates/')){
      if(route.request().method()==='DELETE'){deletes.push(path);plates=plates.filter(p=>!path.endsWith(p.id));return route.fulfill({status:204});}
      return route.fulfill({json:plates.find(p=>path.endsWith(p.id))});
    }
    if(path==='/api/printers')return route.fulfill({json:printers});
    if(path==='/api/queue'){
      if(route.request().method()==='POST')commands.push({printer:url.searchParams.get('printer_id'),command:route.request().postDataJSON()});
      return route.fulfill({json:{epoch:'e',generation:commands.length,request_id:'request',waiting:[],current:null,admission:{allowed,plate_version:1,reason:allowed?null:'No confirmed AMS slot contains the plate material'}}});
    }
    return route.fulfill({json:[]});
  });
  return {printers,commands,deletes,hold:()=>{allowed=false;}};
}
for(const colorScheme of ['dark','light'] as const)test(`${colorScheme}: right click, keyboard, admission, edit and logical deletion`,async({page})=>{
  const {printers,commands,hold}=await fixture(page);await page.emulateMedia({colorScheme});await page.setViewportSize({width:375,height:812});await page.goto('/plates');
  const row=page.locator('.plate-row').first(),dialog=page.getByRole('dialog');
  await row.click({button:'right'});await expect(dialog).toBeVisible();await expect(dialog.getByRole('button',{name:'キュー追加',exact:true})).toBeEnabled();
  await expect(page.getByLabel('追加先のプリンター')).toHaveCount(0);await page.keyboard.press('Escape');await expect(row).toBeFocused();
  await row.press('Shift+F10');await expect(dialog).toBeVisible();await page.locator('.scrim').click({position:{x:5,y:5}});await expect(dialog).toHaveCount(0);
  printers.push({id:'p2',name:'P1S 2',machine_profile_key:machine});await row.click({button:'right'});
  await expect(page.getByLabel('追加先のプリンター')).toBeVisible();await page.getByLabel('追加先のプリンター').selectOption('p2');
  await dialog.getByRole('button',{name:'キュー追加',exact:true}).click();await expect(dialog).toHaveCount(0);expect(commands).toHaveLength(1);expect(commands[0].printer).toBe('p2');expect(commands[0].command.action.type).toBe('add');
  hold();await row.click({button:'right'});await expect(dialog.getByText('この実機のAMSに指定材料の装填を確認できません。AMSの材料を確認してください。')).toBeVisible();await expect(dialog.getByRole('button',{name:'キュー追加',exact:true})).toBeDisabled();
  // Disabled controls are skipped by the existing modal's focus trap.
  await dialog.getByRole('button',{name:'削除',exact:true}).focus();await page.keyboard.press('Tab');await expect(dialog.getByRole('button',{name:'閉じる',exact:true})).toBeFocused();
  if(process.env.E2E_EVIDENCE_DIR)await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/plate-menu-${colorScheme}.png`});
  await dialog.getByRole('button',{name:'削除',exact:true}).click();await expect(dialog).toHaveAccessibleName('プレートを削除');await dialog.getByRole('button',{name:'削除',exact:true}).click();await expect(page.locator('.plate-row')).toHaveCount(1);await expect(page.locator('.plate-row')).toBeFocused();
  await page.locator('.plate-row').press('Shift+F10');await dialog.getByRole('button',{name:'編集',exact:true}).click();await expect(page).toHaveURL(/\/plates\/.+\?edit=1$/);
});

test('long press opens the same menu, release does not navigate, scroll cancels it',async({browser})=>{
  const context=await browser.newContext({viewport:{width:375,height:812},hasTouch:true,isMobile:true});const page=await context.newPage();await fixture(page);await page.goto('/plates');
  const row=page.locator('.plate-row').first();await expect(row).toBeVisible();const box=(await row.boundingBox())!;
  const cdp=await context.newCDPSession(page);const x=box.x+100,y=box.y+20;
  await cdp.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x,y}]});
  await expect(page.getByRole('dialog')).toBeVisible();await cdp.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});await expect(page).toHaveURL(/\/plates$/);
  await page.keyboard.press('Escape');
  await cdp.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x,y}]});
  await cdp.send('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[{x,y:y-30}]});
  await page.waitForTimeout(600);await cdp.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});await expect(page.getByRole('dialog')).toHaveCount(0);
  await context.close();
});

for(const colorScheme of ['dark','light'] as const)for(const width of [375,900])test(`${colorScheme} ${width}: full-width menu and cancellable destructive confirmation`,async({page})=>{
  const {deletes,commands,hold}=await fixture(page);await page.emulateMedia({colorScheme});await page.setViewportSize({width,height:812});await page.goto('/plates');
  const row=page.locator('.plate-row').first();await row.click({button:'right'});const dialog=page.getByRole('dialog');
  const add=dialog.getByRole('button',{name:'キュー追加',exact:true}),edit=dialog.getByRole('button',{name:'編集',exact:true}),remove=dialog.getByRole('button',{name:'削除',exact:true});
  await expect(add).toBeEnabled();
  if(process.env.E2E_EVIDENCE_DIR)await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/menu-${width}-${colorScheme}.png`});
  const boxes=await Promise.all([add,edit,remove].map(b=>b.boundingBox()));
  expect(Math.abs(boxes[0]!.width-boxes[1]!.width)).toBeLessThan(1);expect(Math.abs(boxes[2]!.width-boxes[1]!.width)).toBeLessThan(1);
  const color=await remove.evaluate(e=>getComputedStyle(e).color);expect(color).toBe(colorScheme==='dark'?'rgb(255, 107, 107)':'rgb(156, 43, 29)');
  for(const hover of [false,true]){
    if(hover)await remove.hover();else await page.mouse.move(0,0);
    const contrast=await remove.evaluate(e=>{
      const style=getComputedStyle(e);
      const luminance=(rgb:string)=>rgb.match(/[\d.]+/g)!.slice(0,3).map(Number).map(c=>c/255).map(c=>c<=0.04045?c/12.92:((c+0.055)/1.055)**2.4).reduce((n,c,i)=>n+c*[0.2126,0.7152,0.0722][i],0);
      const a=luminance(style.color),b=luminance(style.backgroundColor);return (Math.max(a,b)+0.05)/(Math.min(a,b)+0.05);
    });expect(contrast).toBeGreaterThanOrEqual(4.5);
  }
  await page.keyboard.press('Tab');await remove.focus();expect(await remove.evaluate(e=>getComputedStyle(e).outlineWidth)).toBe('2px');
  await remove.click();await expect(dialog).toHaveAccessibleName('プレートを削除');await expect(dialog.getByText('「保存プレート 1」を削除しますか？')).toBeVisible();
  await expect(dialog.getByRole('button',{name:'キャンセル'})).toBeFocused();expect(deletes).toHaveLength(0);
  await page.keyboard.press('Enter');await expect(dialog).toHaveAccessibleName('保存プレート 1');await expect(remove).toBeFocused();expect(deletes).toHaveLength(0);
  await remove.click();await page.keyboard.press('Escape');await expect(remove).toBeFocused();expect(deletes).toHaveLength(0);
  await remove.click();await dialog.getByRole('button',{name:'閉じる',exact:true}).click();await expect(remove).toBeFocused();expect(deletes).toHaveLength(0);
  await remove.click();if(process.env.E2E_EVIDENCE_DIR)await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/confirm-${width}-${colorScheme}.png`});
  await dialog.getByRole('button',{name:'削除',exact:true}).click();await expect(page.locator('.plate-row')).toHaveCount(1);await expect(page.locator('.plate-row')).toBeFocused();expect(deletes).toHaveLength(1);expect(commands).toHaveLength(0);
  hold();await page.locator('.plate-row').press('Shift+F10');await expect(add).toBeDisabled();expect(Math.abs((await add.boundingBox())!.width-(await edit.boundingBox())!.width)).toBeLessThan(1);
});

 test('pending deletion sends once, holds keyboard focus and failure returns to cancel',async({page})=>{
  await fixture(page);let count=0,release!:()=>void;const held=new Promise<void>(resolve=>release=resolve);
  await page.route('**/api/plates/*',async route=>{
    if(route.request().method()!=='DELETE')return route.fallback();count++;await held;await route.fulfill({status:503,json:{error:'一時的な保存エラー'}});
  });
  await page.goto('/plates');await page.locator('.plate-row').first().click({button:'right'});const dialog=page.getByRole('dialog');
  await dialog.getByRole('button',{name:'削除',exact:true}).click();
  await dialog.getByRole('button',{name:'削除',exact:true}).evaluate((e:HTMLButtonElement)=>{e.click();e.click();});
  await expect.poll(()=>count).toBe(1);await expect(dialog.getByRole('button',{name:'キャンセル'})).toBeDisabled();await expect(dialog.getByRole('button',{name:'閉じる'})).toBeDisabled();expect(await dialog.getByRole('button',{name:'削除中…',exact:true}).evaluate(e=>getComputedStyle(e).opacity)).toBe('0.5');
  await page.keyboard.press('Escape');await expect(dialog).toBeVisible();await page.keyboard.press('Tab');await expect(dialog).toBeFocused();
  release();await expect(dialog.getByRole('alert')).toBeVisible();await expect(dialog.getByRole('button',{name:'キャンセル'})).toBeFocused();
  await dialog.getByRole('button',{name:'キャンセル'}).click();await expect(page.locator('.plate-row')).toHaveCount(2);expect(count).toBe(1);
 });
 test('long names and 200 percent text keep menu and confirmation within the viewport',async({page})=>{
  await fixture(page,'baseplate_front_'.repeat(10));await page.setViewportSize({width:375,height:812});await page.goto('/plates');await page.addStyleTag({content:':root {--fs-xs:24px;--fs-sm:28px;--fs-md:30px;--fs-lg:32px;--fs-xl:34px;}'});
  await page.locator('.plate-row').first().click({button:'right'});const dialog=page.getByRole('dialog');
  expect(await dialog.evaluate(e=>e.scrollWidth<=e.clientWidth)).toBe(true);
  await dialog.getByRole('button',{name:'削除',exact:true}).click();expect(await dialog.evaluate(e=>e.scrollWidth<=e.clientWidth)).toBe(true);
  await expect(dialog.getByRole('button',{name:'キャンセル'})).toBeFocused();await page.keyboard.press('Enter');await expect(dialog.getByRole('button',{name:'削除',exact:true})).toBeFocused();
 });
