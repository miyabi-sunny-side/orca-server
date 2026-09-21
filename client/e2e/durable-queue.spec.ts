import { test, expect } from '@playwright/test';
const machine='Bambu Lab P1S 0.4 nozzle', mini='Bambu Lab A1 mini 0.2 nozzle', quality='Standard', bed='Textured PEI Plate';
for(const colorScheme of ['dark','light'] as const) {
  test(`${colorScheme}: detail adds directly and preserves an unknown request through reload`,async({page})=>{
    await page.setViewportSize({width:375,height:812});await page.emulateMedia({colorScheme});
    const plate={id:'11111111-1111-4111-8111-111111111111',version:1,name:'繰り返し印刷するプレート',models:[{id:'item',name:'part.stl',source:'part.stl',quantity:10}],conditions:{required_machine_profile_key:machine,filament_id:'white',process_profile_key:quality,bed_type:bed}};
    const printers=[{id:'p1',name:'P1S',machine_profile_key:machine},...[3,1,2].map(n=>({id:`a${n}`,name:`A1 mini ${n}`,machine_profile_key:mini}))];
    let requests:any[]=[],waiting:any[]=[],reads=0,fail=true;
    await page.route('**/api/**',async route=>{
      const path=new URL(route.request().url()).pathname;
      if(path==='/api/plates/11111111-1111-4111-8111-111111111111') return route.fulfill({json:plate});
      if(path==='/api/printers')return route.fulfill({json:printers});
      if(path==='/api/filaments')return route.fulfill({json:[{id:'white',name:'PLA 白',vendor:'Fixture',material:'PLA'}]});
      if(path==='/api/queue'){
        if(route.request().method()==='POST'){
          const data=route.request().postDataJSON();requests.push(data);
          if(!waiting.length)waiting.push({id:'job',plate_id:'11111111-1111-4111-8111-111111111111'});
          if(fail){fail=false;return route.abort('failed');}
        }
        reads++;
        return route.fulfill({json:{epoch:'epoch',generation:waiting.length,request_id:`request-${reads}`,waiting,current:null,allowed:{next:false},admission:{allowed:true,plate_version:1,reason:null}}});
      }
      return route.fulfill({json:[]});
    });
    await page.goto('/plates/11111111-1111-4111-8111-111111111111');
    const add=page.getByRole('button',{name:'印刷キューへ',exact:true});
    await expect(add).toBeEnabled();await expect(page.getByLabel('追加先のプリンター')).toHaveCount(0);
    await add.evaluate((button:HTMLButtonElement)=>{button.click();button.click();});
    await expect(page.getByText('追加の結果が不明です。キューを確認し、同じ要求の結果を再確認してください。')).toBeVisible();
    expect(requests).toHaveLength(1);expect(requests[0].action).toEqual({type:'add',plate_id:'11111111-1111-4111-8111-111111111111',plate_version:1});
    await page.reload();await expect(add).toBeDisabled();
    await page.getByRole('button',{name:'同じ要求を再確認'}).click();
    await expect(page.getByRole('status').filter({hasText:'キューに追加しました'})).toBeVisible();
    expect(requests).toHaveLength(2);expect(requests[1]).toEqual(requests[0]);expect(waiting).toHaveLength(1);
    await expect(page).toHaveURL(/\/plates\/11111111-1111-4111-8111-111111111111$/);
    await expect(page.getByLabel('使用するAMSスロット')).toHaveCount(0);
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    // The same machine profile can match three physical printers; remember only a valid choice.
    plate.conditions.required_machine_profile_key=mini;
    await page.reload();await expect(page.getByLabel('追加先のプリンター')).toHaveValue('a1');
    await expect(page.getByLabel('追加先のプリンター').locator('option')).toHaveCount(3);
    await page.getByLabel('追加先のプリンター').selectOption('a2');await page.reload();
    await expect(page.getByLabel('追加先のプリンター')).toHaveValue('a2');
    printers.splice(printers.findIndex(p=>p.id==='a2'),1);await page.reload();
    await expect(page.getByLabel('追加先のプリンター')).toHaveValue('a1');
    if(process.env.E2E_EVIDENCE_DIR) await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/plate-direct-${colorScheme}.png`,fullPage:true});
  });
}
