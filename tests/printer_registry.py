"""Exercise persistent CRUD, machine selection and isolated Bambu connections.

python3 tests/printer_registry.py BINARY ORCA_APPDIR OUTPUT_DIR
"""
import copy
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from printer_mqtt import SECRET, until, certificate
from printer_start import ARTIFACT, PrintBroker, Ftps, REPO


def main():
    binary, appdir, output = map(lambda p: Path(p).resolve(), sys.argv[1:4])
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='orca-registry-') as directory:
        tmp=Path(directory)
        certificate(tmp,'printer','v1')
        pem=(tmp/'printer.pem').read_text()
        brokers=[PrintBroker(tmp/'printer.pem',tmp/'printer.key',f'PRINTER{i}') for i in range(2)]
        ftps=[Ftps(tmp/'printer.pem',tmp/'printer.key') for _ in range(2)]
        full=json.loads((REPO/'tests/fixtures/p1_status.json').read_text())
        plate_id, revision=str(uuid.uuid4()),str(uuid.uuid4())
        store=tmp/'plates'; artifacts=store/plate_id/'revisions'/revision; artifacts.mkdir(parents=True)
        (artifacts/'0.stl').write_bytes((REPO/'tests/fixtures/cube.stl').read_bytes())
        (artifacts/'print.gcode.3mf').write_bytes(ARTIFACT)
        plate=dict(format_version=1,id=plate_id,revision=revision,name='Routing cube',settings={},
            models=[dict(name='cube.stl',path=f'revisions/{revision}/0.stl',source=None)],project=None,print=f'revisions/{revision}/print.gcode.3mf')
        (store/plate_id/'plate.json').write_text(json.dumps(plate))
        with socket.socket() as sock: sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
        env={k:v for k,v in os.environ.items() if not k.startswith('P1_') and k not in ('ORCA_APPDIR','SCAD_LIVE_URL')}
        env.update(PORT=str(port),PLATES_DIR=str(store),ORCA_APPDIR=str(appdir),LOG_LEVEL='debug')
        process=None;log=(output/'server.log').open('w')
        def api(path, body=None, method=None, expected=200):
            request=urllib.request.Request(f'http://127.0.0.1:{port}{path}',data=json.dumps(body).encode() if body is not None else None,
                method=method,headers={'Content-Type':'application/json'})
            try: response=urllib.request.urlopen(request,timeout=10)
            except urllib.error.HTTPError as e: response=e
            with response:
                raw=response.read()
                assert response.status==expected,(path,response.status,raw)
                assert SECRET.encode() not in raw and b'BEGIN CERTIFICATE' not in raw
                return json.loads(raw) if raw else None
        def start():
            nonlocal process
            process=subprocess.Popen([str(binary)],cwd=tmp,env=env,stdout=log,stderr=log)
            until(lambda: urllib.request.urlopen(f'http://127.0.0.1:{port}/healthz',timeout=1).status==200,30)
        def stop():
            process.terminate();process.wait(timeout=10)
        def status(id): return api(f'/api/printer/status?printer_id={id}')
        def queue(id): return api(f'/api/queue?printer_id={id}')
        def command(id,action,expected=200):
            view=queue(id)
            return api(f'/api/queue?printer_id={id}',dict(epoch=view['epoch'],generation=view['generation'],request_id=view['request_id'],action=action),expected=expected)
        try:
            start()
            assert api('/api/printers')==[]
            machines=api('/api/printers/profiles')
            a1='Bambu Lab A1 mini 0.2 nozzle';p1='Bambu Lab P1S 0.4 nozzle'
            assert any(m['key']==a1 and m['nozzle_diameter']=='0.2' for m in machines)
            a1_profiles=api('/api/slicer/profiles?machine='+urllib.parse.quote(a1))
            p1_profiles=api('/api/slicer/profiles')
            assert a1_profiles['defaults']['machine']==a1
            assert p1_profiles['defaults']['process'] not in a1_profiles['processes']
            api('/api/slicer/profiles?machine=unknown',expected=400)
            if os.environ.get('REGISTRY_BROWSER'):
                browser_env=dict(env,E2E_BASE_URL=f'http://127.0.0.1:{port}',E2E_EVIDENCE_DIR=str(output/'browser'),
                    E2E_REGISTRY_CERT=str(tmp/'printer.pem'),E2E_REGISTRY_SETTINGS=json.dumps(dict(
                        name='UI second',host='127.0.0.1',serial='UISECOND',access_code=SECRET,tls_certificate=pem,
                        machine_profile_key=p1,default_process_profile_key=p1_profiles['defaults']['process'],
                        bed_type=p1_profiles['defaults']['bed'],nozzle_material='unknown')))
                subprocess.run(['npm','run','test:e2e','--','--workers=1'],cwd=REPO/'client',env=browser_env,check=True)
            settings=[];ids=[]
            for i, profiles in enumerate([p1_profiles,a1_profiles]):
                d=dict(name=f'Printer {i}',host='127.0.0.1',serial=f'PRINTER{i}',access_code=SECRET,tls_certificate=pem,
                    machine_profile_key=profiles['printer'],default_process_profile_key=profiles['defaults']['process'],bed_type=profiles['defaults']['bed'],nozzle_material='unknown',mqtt_port=brokers[i].port,ftps_port=ftps[i].port,start_timeout_secs=60)
                saved=api('/api/printers',d,expected=201); ids.append(saved['id']);settings.append(d)
            api('/api/printers',settings[0],expected=409)
            for broker in brokers: until(lambda: len(broker.requests)==1)
            for i, broker in enumerate(brokers):
                report=copy.deepcopy(full);report['print']['mc_percent']=i*17
                broker.send(report)
                until(lambda: status(ids[i])['ready_to_print'])
            assert status(ids[0])['print']['percent']==0 and status(ids[1])['print']['percent']==17
            api('/api/printer/status',expected=409)
            api('/api/printer/status?printer_id=missing',expected=404)
            material=api('/api/filaments',dict(name='PLA for routing',vendor='Fixture',material='PLA',color='FFFFFFFF',bambu_filament_id=None),expected=201)
            api(f'/api/filaments/{material["id"]}/settings',dict(machine_profile_key=p1,base_profile_key=p1_profiles['defaults']['filament'],overrides_json={}),expected=201)
            slots=[]
            for id in ids:
                slot=next(s for s in api(f'/api/printers/{id}/ams')['slots'] if s['ams_id']==0 and s['slot_index']==0)
                api(f'/api/printers/{id}/ams/{slot["id"]}',dict(revision=slot['revision'],filament_id=material['id']),'PUT',204);slots.append(slot)
            spec=dict(ams_slot_id=slots[1]['id'],filament_id=material['id'],required_machine_profile_key=p1,process_profile_key=p1_profiles['defaults']['process'],bed_type=p1_profiles['defaults']['bed'])
            add=dict(type='add',plate_id=plate_id,specification=spec)
            command(ids[0],add,400) # A slot belongs to exactly one printer.
            view=command(ids[1],add);job=view['waiting'][0]
            assert job['hold_reason'] and not view['allowed']['next']
            command(ids[1],dict(type='next',expected_job=job['id'],removed_job=None,cleared=True),409)
            changed=copy.deepcopy(settings[1]);changed.update(machine_profile_key=p1,default_process_profile_key=p1_profiles['defaults']['process'],access_code='',tls_certificate='')
            api('/api/printers/'+ids[1],changed,'PUT') # Waiting jobs do not prevent a registered nozzle change.
            until(lambda:len(brokers[1].requests)==2);brokers[1].send(full)
            until(lambda:status(ids[1])['ready_to_print'])
            view=queue(ids[1]);assert len(view['waiting'])==1 and queue(ids[0])['waiting']==[]
            assert view['waiting'][0]['required_machine_profile_key']==p1
            api('/api/printers/'+ids[1],method='DELETE',expected=409)
            changed['name']='Second edited';api('/api/printers/'+ids[1],changed,'PUT')
            command(ids[1],dict(type='next',expected_job=job['id'],removed_job=None,cleared=True))
            api('/api/printers/'+ids[1],settings[1],'PUT',409)
            spec=copy.deepcopy(spec);spec['ams_slot_id']=slots[0]['id']
            other=command(ids[0],dict(type='add',plate_id=plate_id,specification=spec))['waiting'][0]
            command(ids[0],dict(type='next',expected_job=other['id'],removed_job=None,cleared=True))
            until(lambda:len(brokers[1].prints)==len(brokers[0].prints)==1,60)
            assert [len(f.uploads) for f in ftps]==[1,1]
            for id,peer in zip(ids,ftps):
                current=queue(id)['current'];assert peer.contents[0]==(store/current['artifact_path']/'print.gcode.3mf').read_bytes()
                api('/api/printers/'+id,method='DELETE',expected=409)
            stop()
            env['P1_IP']='not-a-valid-IP-ignored-after-initialization'
            start()
            listed=api('/api/printers')
            assert set(d['id'] for d in listed)==set(ids)
            assert next(d for d in listed if d['id']==ids[1])['name']=='Second edited'
            time.sleep(.5)
            assert [len(b.prints) for b in brokers]==[1,1], 'restart replayed a print'
            for id,broker in zip(ids,brokers):
                api('/api/printers/'+id,method='DELETE',expected=409)
                until(lambda:len(broker.requests)>=2)
                broker.send(full);until(lambda:status(id)['ready_to_print'])
                job=queue(id)['current'];assert job['state']=='needs_attention'
                command(id,dict(type='discard',expected_job=job['id'],cleared=True))
                api('/api/printers/'+id,method='DELETE',expected=204)
            stop();start();assert api('/api/printers')==[], 'deleted registry was reimported'
            assert (store/'orca.sqlite3').stat().st_mode & 0o077 == 0
        finally:
            if process is not None and process.poll() is None: stop()
            log.close()
            for peer in ftps+brokers: peer.close()
        assert SECRET not in (output/'server.log').read_text()
        (output/'result.json').write_text(json.dumps(dict(crud_restart=True,one_time_environment=True,machine_nozzle_profiles=True,separate_status=True,separate_queues=True,both_print_destinations=True,in_use_guard=True,no_implicit_target=True,no_secret_response=True,no_restart_replay=True),indent=2))

if __name__=='__main__': main()
