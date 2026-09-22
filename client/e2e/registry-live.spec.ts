import { expect, test } from "@playwright/test";
import { readFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

test("persistent printer CRUD, nozzle-specific controls and selected queue", async ({page,request})=>{
  const settings=JSON.parse(process.env.E2E_REGISTRY_SETTINGS!);
  const pem=readFileSync(process.env.E2E_REGISTRY_CERT!,"utf8");
  const output=process.env.E2E_EVIDENCE_DIR!;
  mkdirSync(output,{recursive:true});
  await page.goto('/printers');
  await expect(page.getByText('プリンターを追加すると、接続状態の確認と印刷ができます。')).toBeVisible();
  await page.getByRole('link',{name:'追加',exact:true}).click();
  await page.getByLabel('名前',{exact:true}).fill('作業部屋のプリンター・細かい部品のための長い名前');
  await page.getByLabel('機種・装着ノズル径').selectOption('Bambu Lab A1 mini 0.2 nozzle');
  await expect(page.getByLabel('既定の工程')).toHaveValue('0.10mm Standard @BBL A1M 0.2 nozzle');
  expect(await page.getByLabel('既定の工程').locator('option').allTextContents()).not.toContain('0.20mm Standard @BBL X1C');
  await page.getByLabel('ノズル材質').selectOption('stainless_steel');
  await page.getByLabel('IPアドレス').fill('127.0.0.1');
  await page.getByLabel('シリアル番号').fill('UIREGISTRY');
  await page.getByLabel('LANアクセスコード').fill(settings.access_code);
  await page.getByLabel('TLS証明書（PEM）').fill(pem);
  await page.getByText('接続の詳細',{exact:true}).click();
  await page.getByLabel('MQTTポート').fill('1');
  await page.getByLabel('FTPSポート').fill('1');
  await page.getByRole('heading',{name:'プリンターを追加'}).click();
  for(const scheme of ['dark','light'] as const){
    await page.emulateMedia({colorScheme:scheme});
    await page.setViewportSize({width:375,height:812});
    await page.screenshot({path:join(output,`form-${scheme}.png`),fullPage:true});
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  }
  expect(await page.locator('form').evaluate((form:HTMLFormElement)=>Array.from(form.elements).filter((e:any)=>e.willValidate && !e.validity.valid).map((e:any)=>({name:e.outerHTML,message:e.validationMessage})))).toEqual([]);
  await page.getByRole('button',{name:'保存',exact:true}).click();
  await expect(page).toHaveURL(/\/printers$/);
  const saved=(await (await request.get('/api/printers')).json())[0];
  expect(saved.machine_profile_key).toBe('Bambu Lab A1 mini 0.2 nozzle');
  expect(saved.access_code).toBeUndefined();
  for(const scheme of ['dark','light'] as const){
    await page.emulateMedia({colorScheme:scheme});
    for(const width of [320,375,900]){
      await page.setViewportSize({width,height:812});
      await expect(page.getByText('Bambu Lab A1 mini · 0.2 mm')).toBeVisible();
      expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
      await page.screenshot({path:join(output,`list-${scheme}-${width}.png`),fullPage:true});
    }
  }
  await page.getByRole('link',{name:'設定を編集'}).click();
  await expect(page.getByLabel('LANアクセスコード')).toHaveValue('');
  await expect(page.getByLabel('TLS証明書（PEM）')).toHaveValue('');
  await page.getByLabel('名前',{exact:true}).fill('Edited printer');
  await page.getByLabel('機種・装着ノズル径').selectOption('Bambu Lab P1S 0.4 nozzle');
  await expect(page.getByLabel('既定の工程')).toHaveValue('0.20mm Standard @BBL X1C');
  await page.route('**/api/printers/'+saved.id,route=>route.request().method()==='PUT' ? route.fulfill({status:409,json:{error:'in use'}}) : route.continue());
  await page.getByRole('button',{name:'保存',exact:true}).click();
  await expect(page.getByRole('alert')).toContainText('機器が使用中');
  await expect(page.getByLabel('名前',{exact:true})).toHaveValue('Edited printer');
  await page.unroute('**/api/printers/'+saved.id);
  await page.getByRole('button',{name:'保存',exact:true}).click();
  await expect(page).toHaveURL(/\/printers$/);
  await page.getByRole('link',{name:'印刷キュー',exact:true}).click();
  await expect(page).toHaveURL(new RegExp('printer_id='+saved.id));
  await expect(page.getByRole('heading',{name:'Edited printer',exact:true})).toBeVisible();
  const another=await (await request.post('/api/printers',{data:{...settings,serial:'UISECOND',name:'Another printer',access_code:settings.access_code,tls_certificate:pem,mqtt_port:1,ftps_port:1}})).json();
  await page.reload();
  await expect(page.getByRole('combobox',{name:'表示するプリンター',exact:true})).toHaveValue(saved.id);
  await page.getByRole('combobox',{name:'表示するプリンター',exact:true}).selectOption(another.id);
  await expect(page).toHaveURL(new RegExp('printer_id='+another.id));
  await expect(page.getByText('読み込んでいます…',{exact:false})).toHaveCount(0);
  await page.goto('/printers/'+saved.id);
  await expect(page.getByLabel('名前',{exact:true})).toHaveValue('Edited printer');
  await page.setViewportSize({width:375,height:812});
  const originalSize=await page.getByLabel('名前',{exact:true}).evaluate(el=>parseFloat(getComputedStyle(el).fontSize));
  await page.evaluate(()=>{
    const sizes=Array.from(document.querySelectorAll<HTMLElement>('body, body *')).map(el=>[el,parseFloat(getComputedStyle(el).fontSize)] as const);
    for(const [el,size] of sizes) el.style.fontSize=`${size*2}px`;
  });
  expect(await page.getByLabel('名前',{exact:true}).evaluate(el=>parseFloat(getComputedStyle(el).fontSize))).toBe(originalSize*2);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  await page.getByLabel('名前',{exact:true}).focus();await page.keyboard.press('Tab');
  await expect(page.getByLabel('機種・装着ノズル径')).toBeFocused();
  await page.screenshot({path:join(output,'edit-text-200.png'),fullPage:true});
  page.once('dialog',dialog=>dialog.accept());
  await page.getByRole('button',{name:'プリンターを削除'}).click();
  await expect(page).toHaveURL(/\/printers$/);
  expect((await request.delete('/api/printers/'+another.id)).status()).toBe(204);
  await page.reload();
  await expect(page.getByText('プリンターを追加すると、接続状態の確認と印刷ができます。')).toBeVisible();
});
