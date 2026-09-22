import {materialButton,selectMaterial,expectMaterial} from './plate-material';
import {test,expect} from '@playwright/test';
import {mkdirSync} from 'node:fs';
const ctx=JSON.parse(process.env.E2E_DEFAULTS_CONTEXT ?? '{}');
test('saved defaults start complete and manual material survives at desktop and narrow widths',async({page,request})=>{
  test.skip(!!ctx.devices);test.setTimeout(60_000); mkdirSync(process.env.E2E_EVIDENCE_DIR!,{recursive:true});
  const fields=()=>[page.getByLabel('要求する機種・ノズル'),materialButton(page),page.getByLabel('工程（品質）'),page.getByRole('combobox',{name:'ビルドプレート',exact:true})];
  for (const [width,colorScheme] of [[320,'dark'],[900,'light']] as const){
    await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme});
    await page.goto('/plates/new');await page.getByRole('checkbox').check();await page.getByRole('button',{name:'構成を確認（1）'}).click();
    for(const [i,value] of [ctx.machine,ctx.first,ctx.process,ctx.bed].entries()) { if(i===1) await expectMaterial(page,value); else await expect(fields()[i]).toHaveValue(value); }
    await expect(page.getByText('未設定でも保存できます。',{exact:false})).toHaveCount(0);
    await page.getByLabel('プレート名',{exact:true}).fill(`Defaults ${width}`);await page.getByLabel('parts/cube.stl の個数').fill('10');
    if(width===900) await selectMaterial(page,ctx.second);
    await page.mouse.move(0,0); await page.keyboard.press('Escape');
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/defaults-${width}-${colorScheme}.png`,fullPage:true});
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(/\/plates\/[0-9a-f-]{36}$/);
    const id=new URL(page.url()).pathname.split('/')[2];const saved=await (await request.get('/api/plates/'+id)).json();
    expect(saved.conditions).toEqual({required_machine_profile_key:ctx.machine,filament_id:width===900?ctx.second:ctx.first,process_profile_key:ctx.process,bed_type:ctx.bed,sparse_infill_pattern:'adaptivecubic',sparse_infill_density:15,wall_loops:2,brim_enabled:false,support_enabled:false,support_interface_filament_id:null});
    expect(saved.models[0].quantity).toBe(10);
    await page.getByRole('button',{name:'構成を編集'}).click();
    await expectMaterial(page,saved.conditions.filament_id);
  }
  const old=await (await request.get('/api/plates/'+ctx.old)).json();
  const cloneResponse=await request.post('/api/plates/'+ctx.old+'/duplicate',{data:{name:'未設定条件の複製'}});expect(cloneResponse.status()).toBe(201);
  const clone=await cloneResponse.json();expect(clone.conditions).toEqual(old.conditions);
  for(const target of [old,clone]) {
    await page.goto('/plates/'+target.id);await page.getByRole('button',{name:'構成を編集'}).click();
    for(const [i,value] of ['',null,'',''].entries()) { if(i===1) await expectMaterial(page,null); else await expect(fields()[i]).toHaveValue(value!); }
    expect(await (await request.get('/api/plates/'+target.id)).json()).toEqual(target);
    await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page.getByRole('button',{name:'構成を編集'})).toBeVisible();
    const saved=await (await request.get('/api/plates/'+target.id)).json();expect(saved.version).toBe(target.version+1);expect(saved.conditions).toEqual(target.conditions);
  }
  const updated=await (await request.get('/api/plates/'+ctx.old)).json();
  await page.goto('/printers');await expect(page.getByLabel('新規プレートの初期値に使うプリンター')).toHaveValue('p1');
  await page.getByRole('link',{name:'設定を編集'}).click();await page.getByLabel('プレート種類').selectOption('High Temp Plate');
  await page.getByRole('button',{name:'保存',exact:true}).click();await expect(page).toHaveURL(/\/printers$/);
  await page.reload();await expect(page.getByLabel('新規プレートの初期値に使うプリンター')).toHaveValue('p1');
  await page.goto('/plates/new');await page.getByRole('checkbox').check();await page.getByRole('button',{name:'構成を確認（1）'}).click();await expect(fields()[3]).toHaveValue('High Temp Plate');
  expect(await (await request.get('/api/plates/'+ctx.old)).json()).toEqual(updated);
});

test('default printer selection persists across reload among same-model printers',async({page})=>{
  test.skip(!ctx.devices);
  for(const [width,colorScheme] of [[320,'dark'],[900,'light']] as const){
    await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme});
    await page.goto('/printers');const select=page.getByLabel('新規プレートの初期値に使うプリンター');
    await select.selectOption(ctx.devices[width===320?0:1]);await expect(page.getByRole('status')).toHaveText('初期値に使うプリンターを保存しました。');
    await page.reload();await expect(select).toHaveValue(ctx.devices[width===320?0:1]);
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`${process.env.E2E_EVIDENCE_DIR}/default-printer-${width}-${colorScheme}.png`,fullPage:true});
  }
});
