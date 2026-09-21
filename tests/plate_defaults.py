"""Saved creation defaults against disposable SQLite/HTTP/SCAD and printer peers."""
import copy
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import urllib.request
from print_fixture import Rig, MACHINE, PROCESS, FILAMENT, BED, REPO, until
from printer_start import PrintBroker


def main():
    rig = Rig(sys.argv[1], sys.argv[2]); peers = []
    try:
        quality='0.16mm Fixture quality'
        process_path=rig.root/'app/resources/profiles/BBL/process/standard.json'
        extra=json.loads(process_path.read_text()); extra['name']=quality
        process_path.with_name('quality.json').write_text(json.dumps(extra))
        rig.launch(); rig.seed()
        def defaults(): return rig.api('/api/default-settings')
        def settings(pid='p1'):
            return {k:v for k,v in rig.api('/api/printers/'+pid).items() if k not in ('id','status','machine','configuration_error')}
        def create(conditions=None):
            body=dict(name='Initial defaults',models=[dict(name='parts/cube.stl',source='parts/cube.stl',quantity=2)])
            if conditions is not None: body['conditions']=conditions
            return rig.api('/api/plates/import',body,expected=201)
        def edit(plate, conditions):
            return rig.api('/api/plates/'+plate['id'],dict(name=plate['name'],version=plate['version'],models=plate['models'],conditions=conditions),'PUT')
        first=defaults(); assert first['default_printer_id']=='p1'
        assert first['conditions']==dict(required_machine_profile_key=MACHINE,filament_id=rig.materials[0]['id'],process_profile_key=PROCESS,bed_type=BED)
        assert first['reason'] is None
        current=settings(); current['bed_type']='Cool Plate'; requests=len(rig.broker.requests)
        rig.api('/api/printers/p1',current,'PUT')
        initial=create({k:None for k in first['conditions']})
        assert initial['conditions']==dict(first['conditions'],bed_type='Cool Plate')
        # Multipart uploads use the same server-owned defaults as reference creation.
        cube=(REPO/'tests/fixtures/cube.stl').read_bytes(); boundary='defaults-upload'
        body=(f'--{boundary}\r\nContent-Disposition: form-data; name="name"\r\n\r\nUploaded defaults\r\n--{boundary}\r\nContent-Disposition: form-data; name="models"; filename="cube.stl"\r\n\r\n').encode()+cube+f'\r\n--{boundary}--\r\n'.encode()
        req=urllib.request.Request(rig.base+'/api/plates',data=body,headers={'Content-Type':'multipart/form-data; boundary='+boundary})
        with urllib.request.urlopen(req) as response: uploaded=json.load(response)
        assert uploaded['conditions']==initial['conditions']
        rig.api('/api/default-settings',dict(default_printer_id=None),'PUT',403,origin='https://outside.invalid')
        manual=create(dict(filament_id=rig.materials[1]['id'],bed_type='High Temp Plate'))
        assert manual['conditions']==dict(initial['conditions'],filament_id=rig.materials[1]['id'],bed_type='High Temp Plate')
        old=edit(initial,{k:None for k in first['conditions']})
        assert all(v is None for v in old['conditions'].values())
        assert len(rig.broker.requests)==requests and not rig.broker.prints
        # Priority is a print-slot choice, not the initial material's physical slot order.
        slots=rig.api('/api/printers/p1/ams')['slots']; slot0=next(s for s in slots if s['slot_index']==0)
        unsupported=rig.api('/api/filaments',dict(name='No machine setting',vendor='Fixture',material='PLA',color='FFFFFFFF',bambu_filament_id=None),expected=201)
        rig.api(f'/api/printers/p1/ams/{slot0["id"]}',dict(revision=slot0['revision'],filament_id=unsupported['id']),'PUT',204)
        assert defaults()['conditions']['filament_id']==rig.materials[1]['id']
        slot0=next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==0)
        rig.api(f'/api/printers/p1/ams/{slot0["id"]}',dict(revision=slot0['revision'],filament_id=None),'PUT',204)
        assert defaults()['conditions']['filament_id']==rig.materials[1]['id']
        report=copy.deepcopy(rig.full); report['print']['ams']['tray_exist_bits']='1'; rig.broker.send(report)
        until(lambda:defaults()['conditions']['filament_id'] is None)
        assert defaults()['reason']=='material'
        rig.idle(); until(lambda:next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==3)['reported']['present'] is True)
        for index,material in [(0,rig.materials[0]),(3,rig.materials[1])]:
            slot=next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==index)
            rig.api(f'/api/printers/p1/ams/{slot["id"]}',dict(revision=slot['revision'],filament_id=material['id']),'PUT',204)
        if os.environ.get('DEFAULTS_BROWSER'):
            env=dict(os.environ,E2E_BASE_URL=rig.base,E2E_EVIDENCE_DIR=str(rig.output/'browser'),E2E_DEFAULTS_CONTEXT=json.dumps(dict(machine=MACHINE,process=PROCESS,bed='Cool Plate',first=rig.materials[0]['id'],second=rig.materials[1]['id'],old=old['id'])))
            subprocess.run(['npm','run','test:e2e','--','--workers=1'],cwd=REPO/'client',env=env,check=True)
        old=edit(rig.api('/api/plates/'+old['id']),{k:None for k in first['conditions']})
        # A real MCP client goes through the same creation and explicit-update routes.
        subprocess.run(['cargo','test','--locked','--test','mcp','creation_defaults_match_rest','--','--ignored','--nocapture'],cwd=REPO,env={**os.environ,'MCP_FIXTURE_URL':rig.base},check=True)
        # Default edits during an actual isolated active attempt preserve frozen input and connection.
        rig.plate=manual; job=rig.add(); rig.next(job); until(lambda:len(rig.broker.prints)==1)
        rig.report('RUNNING'); rig.phase('printing')
        before=rig.api()['current']; trace=rig.traces(); requests=len(rig.broker.requests); uploads=len(rig.ftp.uploads)
        current=settings(); current['bed_type']='High Temp Plate'; current['default_process_profile_key']=quality; rig.api('/api/printers/p1',current,'PUT')
        assert rig.api()['current']==before and rig.traces()==trace
        assert len(rig.broker.requests)==requests and len(rig.ftp.uploads)==uploads and len(rig.broker.prints)==1
        assert rig.api('/api/plates/'+manual['id'])==rig.plate
        current['machine_profile_key']='Bambu Lab P1S 0.2 nozzle'; rig.api('/api/printers/p1',current,'PUT',409)
        # Same-model devices remain independent; only the selected device supplies loaded materials.
        for n in range(2):
            peer=PrintBroker(rig.root/'trusted.pem',rig.root/'trusted.key',f'DEFAULT{n}'); peers.append(peer)
            config=dict(settings(),name=f'Default {n}',serial=f'DEFAULT{n}',mqtt_port=peer.port,access_code=rig.env['P1_ACCESS_CODE'],tls_certificate=(rig.root/'trusted.pem').read_text())
            device=rig.api('/api/printers',config,expected=201); peer.device=device
            until(lambda:len(peer.requests)>0); peer.send(rig.full)
            until(lambda:rig.api('/api/printer/status?printer_id='+device['id'])['synchronized'])
        chosen=peers[0].device['id']; other=peers[1].device['id']
        assert defaults()['default_printer_id']=='p1'
        if os.environ.get('DEFAULTS_BROWSER'):
            env=dict(os.environ,E2E_BASE_URL=rig.base,E2E_EVIDENCE_DIR=str(rig.output/'browser'),E2E_DEFAULTS_CONTEXT=json.dumps(dict(devices=[chosen,other])))
            subprocess.run(['npm','run','test:e2e','--','--workers=1'],cwd=REPO/'client',env=env,check=True)
        rig.api('/api/default-settings',dict(default_printer_id=chosen),'PUT',204)
        assert defaults()['conditions']['filament_id'] is None
        rig.api('/api/default-settings',dict(default_printer_id='missing'),'PUT',404)
        assert defaults()['default_printer_id']==chosen
        rig.stop(); rig.launch()
        assert defaults()['default_printer_id']==chosen and defaults()['conditions']['filament_id'] is None
        assert rig.api('/api/plates/'+old['id'])==old
        rig.api('/api/printers/'+chosen,method='DELETE',expected=204)
        assert defaults()['default_printer_id'] is None and defaults()['reason']=='printer_selection'
        rig.api('/api/printers/'+other,method='DELETE',expected=204)
        assert defaults()['default_printer_id']=='p1'
        with sqlite3.connect(rig.store/'orca.sqlite3') as c:
            assert c.execute('PRAGMA user_version').fetchone()==(8,)
            assert [r[1] for r in c.execute('PRAGMA table_info(default_settings)')]==['id','default_printer_id']
            assert c.execute('SELECT default_printer_id FROM default_settings').fetchall()==[('p1',)]
        assert len(rig.broker.prints)==1 and all(not p.prints for p in peers)
        (rig.output/'verified.json').write_text(json.dumps(dict(defaults_persist=True,manual_values_preserved=True,update_null_clears=True,current_ams_only=True,active_input_preserved=True,same_model_devices=True,mcp=True),indent=2))
    finally:
        rig.close()
        for peer in peers: peer.close()

def recover_default_process(binary, output):
    rig=Rig(binary,output)
    try:
        rig.launch(); rig.stop()
        with sqlite3.connect(rig.store/'orca.sqlite3') as c:
            c.execute("UPDATE printers SET default_process_profile_key='removed profile' WHERE id='p1'")
        rig.process=subprocess.Popen([rig.binary],cwd=rig.root,env=rig.env,stdout=rig.log,stderr=rig.log)
        until(lambda:urllib.request.urlopen(rig.base+'/healthz',timeout=1).status==200)
        broken=rig.api('/api/printers/p1'); assert broken['configuration_error']
        settings={k:v for k,v in broken.items() if k not in ('id','status','machine','configuration_error')}
        settings['default_process_profile_key']=PROCESS
        requests=len(rig.broker.requests)
        fixed=rig.api('/api/printers/p1',settings,'PUT')
        assert fixed['configuration_error'] is None, fixed['configuration_error']
        until(lambda:len(rig.broker.requests)>requests)
        rig.idle(); assert not rig.broker.prints and not rig.ftp.uploads
    finally: rig.close()


if __name__=='__main__':
    main()
    recover_default_process(sys.argv[1],str(Path(sys.argv[2])/'recovery'))
