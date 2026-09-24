import {materialButton,selectMaterial} from './plate-material';
import {test,expect} from '@playwright/test';
test('late defaults preserve deliberate selections and clearing, without repeated refresh',async({page})=>{
  const machine='Bambu Lab P1S 0.4 nozzle',process='Standard';let reads=0;
  let release!:()=>void;const gate=new Promise<void>(resolve=>{release=resolve;});
  const defaults={default_printer_id:'p1',conditions:{required_machine_profile_key:machine,filament_id:'white',process_profile_key:process,bed_type:'Cool Plate'},reason:null};
  const filaments=['white','blue'].map(id=>({id,name:id,vendor:'Test',material:'PLA'}));
  await page.route('**/api/**',async route=>{
    const path=new URL(route.request().url()).pathname;
    if(path==='/api/default-settings'){reads++;await gate;return route.fulfill({json:defaults});}
    if(path==='/api/scad/model-info')return route.fulfill({json:{roles:['primary']}});
    if(path==='/api/scad/models')return route.fulfill({json:['cube.stl']});
    if(path==='/api/printers')return route.fulfill({json:[{id:'p1',machine_profile_key:machine}]});
    if(path==='/api/plate-filaments') {const id=new URL(route.request().url()).searchParams.get('selected_id');return route.fulfill({json:{filaments,selected:filaments.find(f=>f.id===id)??null,selected_state:id?'loaded':'unset',loaded_ids:filaments.map(f=>f.id),printers:[{id:'p1',name:'P1',state:'current',unassigned:false}]}});}
    if(path==='/api/filaments')return route.fulfill({json:filaments});
    if(path.startsWith('/api/filaments/'))return route.fulfill({json:{settings:[{machine_profile_key:machine}]}});
    if(path==='/api/slicer/profiles')return route.fulfill({json:{processes:[process],beds:['Cool Plate','High Temp Plate']}});
    return route.fulfill({status:404,json:{error:'not found'}});
  });
  await page.goto('/plates/new');await page.getByRole('checkbox').check();await page.getByRole('button',{name:'構成を確認（1）'}).click();
  await page.getByLabel('要求する機種・ノズル').selectOption(machine);
  const material=materialButton(page);await selectMaterial(page,'blue');await selectMaterial(page,null);
  await page.getByLabel('工程（品質）').selectOption(process);await page.getByRole('combobox',{name:'ビルドプレート',exact:true}).selectOption('High Temp Plate');
  release();await expect(page.getByText('初期値を読み込んでいます…')).toHaveCount(0);
  await expect(material).toContainText('フィラメントを選択');await expect(page.getByRole('combobox',{name:'ビルドプレート',exact:true})).toHaveValue('High Temp Plate');
  await selectMaterial(page,'blue');await page.getByRole('button',{name:'モデル選択へ'}).click();await page.getByRole('button',{name:'構成を確認（1）'}).click();
  await expect(material).toContainText('blue');expect(reads).toBe(1);
});
