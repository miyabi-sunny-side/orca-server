import {test,expect} from '@playwright/test';
const machine='Bambu Lab P1S 0.4 nozzle';
async function fixture(page:any){
  let plates=[1,2].map(i=>({id:`10000000-0000-4000-8000-${String(i).padStart(12,'0')}`,version:1,name:`保存プレート ${i}`,models:[{id:'model',name:'cube.stl',source:'cube.stl',quantity:1}],conditions:{required_machine_profile_key:machine,filament_id:'white',process_profile_key:'quality',bed_type:'Cool Plate'}}));
  const printers=[{id:'p1',name:'P1S',machine_profile_key:machine}];let allowed=true;const commands:any[]=[];
  await page.route('**/api/**',async(route:any)=>{
    const url=new URL(route.request().url()),path=url.pathname;
    if(path==='/api/plates')return route.fulfill({json:plates});
    if(path.startsWith('/api/plates/')){
      if(route.request().method()==='DELETE'){plates=plates.filter(p=>!path.endsWith(p.id));return route.fulfill({status:204});}
      return route.fulfill({json:plates.find(p=>path.endsWith(p.id))});
    }
    if(path==='/api/printers')return route.fulfill({json:printers});
    if(path==='/api/queue'){
      if(route.request().method()==='POST')commands.push({printer:url.searchParams.get('printer_id'),command:route.request().postDataJSON()});
      return route.fulfill({json:{epoch:'e',generation:commands.length,request_id:'request',waiting:[],current:null,admission:{allowed,plate_version:1,reason:allowed?null:'No confirmed AMS slot contains the plate material'}}});
    }
    return route.fulfill({json:[]});
  });
  return {printers,commands,hold:()=>{allowed=false;}};
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
  page.on('dialog',()=>{throw new Error('Unexpected confirmation');});await dialog.getByRole('button',{name:'削除',exact:true}).click();await expect(page.locator('.plate-row')).toHaveCount(1);await expect(page.locator('.plate-row')).toBeFocused();
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
