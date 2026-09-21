"""Real HTTP/SQLite/profile + isolated MQTT regression.

python3 tests/filament_ams.py BINARY ORCA_APPDIR OUTPUT_DIR
Set FILAMENT_BROWSER=1 to include Chromium CRUD and AMS checks.
"""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import urllib.error
import urllib.parse
import urllib.request
from printer_mqtt import Broker, SECRET, certificate, until

REPO=Path(__file__).resolve().parents[1]


def main():
    binary,appdir,output=(Path(p).resolve() for p in sys.argv[1:4]);output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='orca-filaments-') as directory:
        tmp=Path(directory);certificate(tmp,'printer','v1');pem=(tmp/'printer.pem').read_text()
        brokers=[Broker(tmp/'printer.pem',tmp/'printer.key',f'MATERIAL{i}') for i in range(2)]
        with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
        env={k:v for k,v in os.environ.items() if not k.startswith('P1_') and k not in ('ORCA_APPDIR','SCAD_LIVE_URL')}
        env.update(PORT=str(port),PLATES_DIR=str(tmp/'plates'),ORCA_APPDIR=str(appdir))
        process=None;control=None;log=(output/'server.log').open('w')
        def api(path,body=None,method=None,expected=200):
            req=urllib.request.Request(f'http://127.0.0.1:{port}{path}',data=None if body is None else json.dumps(body).encode(),method=method,headers={'Content-Type':'application/json'})
            try:response=urllib.request.urlopen(req,timeout=10)
            except urllib.error.HTTPError as e:response=e
            with response:
                raw=response.read();assert response.status==expected,(path,response.status,raw)
                assert SECRET.encode() not in raw and b'BEGIN CERTIFICATE' not in raw
                return (json.loads(raw) if response.headers.get_content_type()=="application/json" else raw.decode()) if raw else None
        def start():
            nonlocal process
            process=subprocess.Popen([str(binary)],cwd=tmp,env=env,stdout=log,stderr=log)
            until(lambda:urllib.request.urlopen(f'http://127.0.0.1:{port}/healthz',timeout=1).status==200,30)
        def stop():process.terminate();process.wait(timeout=10)
        def inventory(index=0):return api(f'/api/printers/{ids[index]}/ams')
        def mapping(slot,material,index=0,expected=204):
            return api(f'/api/printers/{ids[index]}/ams/{slot["id"]}',dict(revision=slot['revision'],filament_id=material),'PUT',expected)
        def delta(tray):return dict(print=dict(command='push_status',msg=1,ams=dict(ams=[dict(id='0',tray=[tray])])))
        full=dict(print=dict(command='push_status',msg=0,gcode_state='IDLE',print_error=0,ams=dict(tray_exist_bits='f',ams_exist_bits='1',insert_flag=True,power_on_flag=True,ams=[dict(id='0',tray=[
            dict(id='0',tray_type='PETG',tray_color='FFFFFFFF',nozzle_temp_min=240,nozzle_temp_max=260,remain=-1),
            dict(id='1',tray_type='PLA',tray_info_idx='GFA01',tray_sub_brands='PLA Matte',tray_color='000000FF',tag_uid='SYNTHETICBLACK',remain=42),
            dict(id='2',tray_type='PLA',tray_info_idx='GFA01',tray_sub_brands='PLA Matte',tray_color='FFFFFFFF',tag_uid='SYNTHETICWHITE',remain=-1),
            dict(id='3',tray_type='PETG',tray_color='00AAFFFF',nozzle_temp_min=220,nozzle_temp_max=260,remain=50)
        ])])))
        try:
            start();assert api('/api/filaments')==[]
            profiles=api('/api/slicer/profiles');machine=profiles['printer'];ids=[]
            for i,b in enumerate(brokers):
                saved=api('/api/printers',dict(name=f'材料確認 {i}',host='127.0.0.1',serial=b.serial,access_code=SECRET,tls_certificate=pem,
                    machine_profile_key=machine,default_process_profile_key=profiles['defaults']['process'],bed_type=profiles['defaults']['bed'],nozzle_material='stainless_steel',mqtt_port=b.port,ftps_port=1),expected=201)
                ids.append(saved['id'])
            materials=[]
            for name,vendor,kind,color,bid in [('ガラス繊維入りPETG','Third party','PETG-GF','FFFFFFFF',None),('PLA Matte 黒','Bambu Lab','PLA','000000FF','GFA01'),('PLA Matte 白','Bambu Lab','PLA','FFFFFFFF','GFA01'),('透明ブルーPETG','Third party','PETG','00AAFFFF',None)]:
                materials.append(api('/api/filaments',dict(name=name,vendor=vendor,material=kind,color=color,bambu_filament_id=bid),expected=201))
            gf,black,white,petg=materials
            settings=[]
            for i,f in enumerate(materials):
                choices=api(f'/api/filaments/{f["id"]}/profiles?machine='+urllib.parse.quote(machine))
                match=next(p for p in choices if ('Bambu PLA Matte @BBL X1C' if i in (1,2) else 'Generic PETG')==p['key'])
                overrides=({} if i in (1,2) else dict(nozzle_temperature_initial_layer=250 if i==0 else 240,nozzle_temperature=240 if i==0 else 220))
                settings.append(api(f'/api/filaments/{f["id"]}/settings',dict(machine_profile_key=machine,base_profile_key=match['key'],overrides_json=overrides),expected=201))
            def data(s):return {k:v for k,v in s.items() if k not in ('id','filament_id')}
            api(f'/api/filaments/{gf["id"]}/settings',data(settings[0]),expected=409)
            bad=data(settings[0]);bad['machine_profile_key']='Bambu Lab A1 mini 0.2 nozzle'
            api(f'/api/filaments/{gf["id"]}/settings',bad,expected=400)
            bad=data(settings[0]);bad['overrides_json']={'filament_start_gcode':'G1 X1'}
            api(f'/api/filaments/{gf["id"]}/settings',bad,expected=422)
            for b in brokers:until(lambda:len(b.requests)==1);b.send(full)
            until(lambda:inventory()['current'] and len(inventory()['slots'])==4)
            until(lambda:inventory(1)['current'])
            slots=inventory()['slots'];assert [s['filament_id'] for s in slots]==[None,black['id'],white['id'],None]
            assert slots[0]['reported']['remaining_percent'] is None
            assert slots[1]['reported']['brand']=='PLA Matte' and slots[1]['detect_on_insert'] is True and slots[1]['detect_on_power_up'] is True
            assert slots[2]['setting']['overrides_json']=={} and slots[2]['setting']['resolved']['nozzle_temperature']
            mapping(slots[0],gf['id']);mapping(slots[3],petg['id'])
            slots=inventory()['slots'];assert slots[0]['filament']['material']=='PETG-GF' and slots[0]['reported']['material']=='PETG'
            assert slots[0]['setting']['resolved']['nozzle_temperature_initial_layer']=='250'
            assert slots[0]['setting']['resolved']['nozzle_temperature']=='240'
            assert slots[3]['reported']['temperature_max']==260 and slots[3]['setting']['resolved']['nozzle_temperature']=='220'
            assert inventory(1)['slots'][0]['filament_id'] is None
            api(f'/api/filaments/{gf["id"]}',method='DELETE',expected=409)
            # Only status snapshot requests are permitted: the peer rejects any feed, heat or RFID command.
            class Control(BaseHTTPRequestHandler):
                def do_POST(self):
                    value=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    if value.get('disconnect'):brokers[0].actions.put('disconnect')
                    elif value.get('full'):brokers[0].send(full)
                    else:brokers[0].send(delta(value))
                    self.send_response(204);self.end_headers()
                def log_message(self,*_):pass
            if os.environ.get('FILAMENT_BROWSER'):
                control=ThreadingHTTPServer(('127.0.0.1',0),Control);threading.Thread(target=control.serve_forever,daemon=True).start()
                browser_env=dict(env,E2E_BASE_URL=f'http://127.0.0.1:{port}',E2E_EVIDENCE_DIR=str(output/'browser'),E2E_FILAMENT_CONTEXT=json.dumps(dict(printer=ids[0],gf=gf['id'],black=black['id'],white=white['id'],petg=petg['id'],machine=machine,control=f'http://127.0.0.1:{control.server_port}')))
                subprocess.run(['npm','run','test:e2e','--','--workers=1'],cwd=REPO/'client',env=browser_env,check=True)
            # A swap away and back while no HTTP client is reading must still invalidate manual mapping.
            brokers[0].send(delta(dict(id='0',tray_color='000000FF')))
            def stored_unassigned():
                with sqlite3.connect(tmp/'plates/orca.sqlite3') as c:
                    return c.execute('SELECT filament_id FROM ams_slots WHERE printer_id=? AND slot_index=0',(ids[0],)).fetchone()==(None,)
            until(stored_unassigned);brokers[0].send(delta(dict(id='0',tray_color='FFFFFFFF')))
            until(lambda:inventory()['slots'][0]['reported']['color']=='FFFFFFFF')
            assert inventory()['slots'][0]['filament_id'] is None
            mapping(slots[0],gf['id'],expected=409)
            mapping(inventory()['slots'][0],gf['id'])
            # A tag swap invalidates a deliberately different manual choice, then resolves the observed white spool.
            mapping(inventory()['slots'][1],gf['id'])
            brokers[0].send(delta(dict(id='1',tray_color='FFFFFFFF',tag_uid='REPLACEMENTWHITE')))
            until(lambda:inventory()['slots'][1]['filament_id']==white['id'])
            brokers[0].send(delta(dict(id='1',tray_info_idx='')))
            until(lambda:inventory()['slots'][1]['filament_id'] is None)
            assert inventory(1)['slots'][1]['filament_id']==black['id']
            # Restart preserves user temperatures and last observation, without calling it current.
            stop();start();assert not inventory()['current'] and len(inventory()['slots'])==4
            assert not any(s['current'] for s in inventory()['slots'])
            mapping(inventory()['slots'][0],gf['id'],expected=409)
            resolved=api(f'/api/filaments/{gf["id"]}')['settings'][0]['resolved'];assert resolved['nozzle_temperature']=='240'
            for b in brokers:until(lambda:len(b.requests)>=2);b.send(full)
            until(lambda:inventory()['current']);assert inventory()['slots'][1]['filament_id']==black['id']
            with sqlite3.connect(tmp/'plates/orca.sqlite3') as c:assert c.execute('PRAGMA user_version').fetchone()==(9,)
            (output/'verified.json').write_text(json.dumps(dict(schema=5,printers=2,filaments=4,initial_layer=250,normal=240,matte_black_white=True,missed_swap_invalidates=True,restart_unconfirmed=True,isolated_commands='status-only'),indent=2))
        finally:
            if process and process.poll() is None:stop()
            if control:control.shutdown();control.server_close()
            for b in brokers:b.close()
            log.close()
        assert SECRET.encode() not in (output/'server.log').read_bytes()
    print('filament / AMS persistence, profiles, mapping and MQTT integration passed')

if __name__=='__main__':main()
