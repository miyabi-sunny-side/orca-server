"""Plate conditions and admission against HTTP/SQLite and four isolated printers."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
from print_fixture import Rig, MACHINE, PROCESS, FILAMENT, BED, REPO, until
from printer_start import PrintBroker


def main():
    rig=Rig(sys.argv[1],sys.argv[2]);peers=[]
    mini='Bambu Lab A1 mini 0.2 nozzle'
    try:
        rig.launch();rig.seed();base=rig.configure();fid=rig.materials[1]['id']
        def save(conditions, plate=None):
            plate=rig.api('/api/plates/'+(plate or base)['id'])
            return rig.api('/api/plates/'+plate['id'],dict(name=plate['name'],version=plate['version'],models=plate['models'],conditions=conditions),'PUT')
        def admission(plate, pid='p1'):
            return rig.api(f'/api/queue?printer_id={pid}&plate_id={plate["id"]}')
        def add(plate,pid='p1',expected=200):
            q=admission(plate,pid)
            return rig.api(f'/api/queue?printer_id={pid}',rig.command(dict(type='add',plate_id=plate['id'],plate_version=plate['version']),q),expected=expected)
        conditions=copy.deepcopy(base['conditions'])
        for key in [None,'required_machine_profile_key','filament_id','process_profile_key','bed_type']:
            missing=dict(conditions)
            if key is None:missing={k:None for k in conditions if k not in ("brim_enabled", "support_enabled")}
            else:missing[key]=None
            if missing['required_machine_profile_key'] is None:missing['process_profile_key']=None
            plate=save(missing)
            assert not admission(plate)['admission']['allowed'];add(plate,expected=409)
            assert not rig.api()['waiting']
        base=save(conditions)
        for key,value,code in [('required_machine_profile_key','unowned machine',409),('filament_id','missing material',404),('process_profile_key','wrong process',400),('bed_type','wrong bed',400)]:
            invalid=dict(conditions);invalid[key]=value
            rig.api('/api/plates/'+base['id'],dict(name=base['name'],version=base['version'],models=base['models'],conditions=invalid),'PUT',code)
            assert rig.api('/api/plates/'+base['id'])==base
        # Loaded data belongs to this selected physical printer, never a sibling of the same model.
        settings={k:v for k,v in rig.api('/api/printers/p1').items() if k not in ('id','status','machine','configuration_error')}
        rig.api(f'/api/filaments/{fid}/settings',dict(machine_profile_key=mini,base_profile_key=FILAMENT,overrides_json={}),expected=201)
        devices=[]
        for n in range(3):
            peer=PrintBroker(rig.root/'trusted.pem',rig.root/'trusted.key',f'MINI{n}');peers.append(peer)
            config=dict(settings,name=f'A1 mini {n+1}',serial=f'MINI{n}',mqtt_port=peer.port,machine_profile_key=mini,access_code=rig.env['P1_ACCESS_CODE'],tls_certificate=(rig.root/'trusted.pem').read_text())
            device=rig.api('/api/printers',config,expected=201);devices.append(device)
            until(lambda:len(peer.requests)>0);peer.send(rig.full)
            until(lambda:rig.api('/api/printer/status?printer_id='+device['id'])['synchronized'])
        assert len(rig.api('/api/printers'))==4
        add(base,devices[0]['id'],409)
        mini_conditions=dict(conditions,required_machine_profile_key=mini)
        mini_plate=rig.api('/api/plates/import',dict(name='mini plate',models=[dict(name='parts/cube.stl',source='parts/cube.stl',quantity=10)],conditions=mini_conditions),expected=201)
        for device in devices:assert not admission(mini_plate,device['id'])['admission']['allowed'];add(mini_plate,device['id'],409)
        device=devices[2];slot=next(s for s in rig.api(f'/api/printers/{device["id"]}/ams')['slots'] if s['slot_index']==3)
        rig.api(f'/api/printers/{device["id"]}/ams/{slot["id"]}',dict(revision=slot['revision'],filament_id=fid),'PUT',204)
        assert admission(mini_plate,device['id'])['admission']['allowed'];add(mini_plate,device['id'])
        for other in devices[:2]:add(mini_plate,other['id'],409)
        # An unknown, explicitly empty or cleared slot cannot pass the shared admission check.
        for bits in ['1','0']:
            report=copy.deepcopy(rig.full);report['print']['ams']['tray_exist_bits']=bits
            rig.broker.send(report);until(lambda:not next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==3)['reported']['present'])
            assert not admission(base)['admission']['allowed'];add(base,expected=409)
        rig.idle();until(lambda:next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==3)['reported']['present'] is True)
        slot=next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==3)
        rig.api(f'/api/printers/p1/ams/{slot["id"]}',dict(revision=slot['revision'],filament_id=fid),'PUT',204)
        busy=copy.deepcopy(rig.full);busy['print']['gcode_state']='RUNNING'
        rig.broker.send(busy);until(lambda:not rig.api()['printer']['ready_to_print'])
        assert admission(base)['admission']['allowed'];add(base)
        assert not rig.broker.prints and all(not p.prints for p in peers)
        # A changed plate between eligibility and POST invalidates both its version and queue fence.
        q=admission(base);request=rig.command(dict(type='add',plate_id=base['id'],plate_version=base['version']),q)
        old_version=base['version'];base=save(dict(conditions,bed_type='High Temp Plate'))
        rig.api(body=request,expected=409)
        add(dict(base,version=old_version),expected=409)
        assert rig.api()['waiting'][0]['bed_type']=='High Temp Plate'
        rig.idle()
        # Load the same material for the browser's arbitrary matching-printer selection.
        for device in devices[:2]:
            slot=next(s for s in rig.api(f'/api/printers/{device["id"]}/ams')['slots'] if s['slot_index']==3)
            rig.api(f'/api/printers/{device["id"]}/ams/{slot["id"]}',dict(revision=slot['revision'],filament_id=fid),'PUT',204)
        if os.environ.get('PLATE_BROWSER'):
            env=dict(os.environ,E2E_BASE_URL=rig.base,E2E_EVIDENCE_DIR=str(rig.output/'browser'),E2E_PLATE_CONTEXT=json.dumps(dict(devices=[d['id'] for d in devices],filament=fid,machine=MACHINE,mini=mini,process=PROCESS,bed=BED)))
            subprocess.run(['npm','run','test:e2e','--','--workers=1'],cwd=REPO/'client',env=env,check=True)
        rig.stop();rig.launch()
        assert rig.api('/api/plates/'+base['id'])==base
        assert not admission(base)['admission']['allowed'];add(base,expected=409)
        assert not rig.broker.prints and all(not p.prints for p in peers)
        (rig.output/'verified.json').write_text(json.dumps(dict(nullable=True,owned_profiles=True,exact_device_ams=True,busy_enqueue=True,stale_plate_refused=True,restart_preserves_conditions=True,stale_ams_refused=True,no_print_commands=True),indent=2))
    finally:
        rig.close()
        for peer in peers:peer.close()
    print('Plate ownership, nullable conditions, exact device admission and no automatic print passed')

if __name__=='__main__':main()
