"""Disposable server, SCAD source and protocol peers shared by print-flow checks.

The small CLI substitute records application inputs; real Orca slicing is verified separately.
"""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import urllib.error
import urllib.parse
import urllib.request
from printer_mqtt import SERIAL, SECRET, certificate, until
from printer_start import PrintBroker, Ftps, REPO

MACHINE = 'Bambu Lab P1S 0.4 nozzle'
PROCESS = '0.20mm Standard @BBL X1C'
FILAMENT = 'Generic PLA High Speed @BBL X1C'
BED = 'Textured PEI Plate'


def fake_slicer(root):
    app = root/'app'; profiles = app/'resources/profiles/BBL'
    machines = [MACHINE, 'Bambu Lab P1S 0.2 nozzle', 'Bambu Lab A1 mini 0.2 nozzle']
    for category in ['machine', 'process', 'filament']:
        (profiles/category).mkdir(parents=True, exist_ok=True)
    for i, machine in enumerate(machines):
        data = dict(name=machine, instantiation='true', nozzle_diameter=['0.4' if i == 0 else '0.2'],
                    printer_model=machine.split(' 0.')[0], default_print_profile=PROCESS,
                    default_filament_profile=[FILAMENT])
        (profiles/'machine'/f'{i}.json').write_text(json.dumps(data))
    (profiles/'process'/'standard.json').write_text(json.dumps(dict(name=PROCESS, instantiation='true', compatible_printers=machines, brim_width='5', brim_object_gap='0.1', sparse_infill_pattern='crosshatch', sparse_infill_density='15%', wall_loops='2', top_shell_layers='5', bottom_shell_layers='3', top_shell_thickness='1', bottom_shell_thickness='0')))
    for i, (name, material) in enumerate([(FILAMENT, 'PLA'), ('Generic PETG', 'PETG')]):
        (profiles/'filament'/f'{i}.json').write_text(json.dumps(dict(name=name, instantiation='true', compatible_printers=machines,
            cool_plate_temp=['35'], cool_plate_temp_initial_layer=['35'], eng_plate_temp=['55'], eng_plate_temp_initial_layer=['55'], hot_plate_temp=['55'], hot_plate_temp_initial_layer=['55'], textured_plate_temp=['55'], textured_plate_temp_initial_layer=['55'], filament_type=[material], nozzle_temperature=['220'], nozzle_temperature_initial_layer=['220'], required_nozzle_HRC=['0'])))
    script = '''#!/usr/bin/env python3
import hashlib, io, json, pathlib, sys, time, zipfile, xml.etree.ElementTree as ET
if '--help' in sys.argv:
    print('OrcaSlicer-2.4.2:'); raise SystemExit()
root=pathlib.Path.cwd()
profiles={name:json.loads((root/(name+'.json')).read_text()) for name in ['printer','process','filament','interface'] if (root/(name+'.json')).exists()}
materials=[profiles[n] for n in ['filament','interface'] if n in profiles]
inputs=[hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.glob('*.stl'))]
with pathlib.Path(TRACE).open('a') as log: log.write(json.dumps(dict(directory=str(root),arguments=sys.argv[1:],profiles=profiles,inputs=inputs))+'\\n')
control=pathlib.Path(FIXTURE_CONTROL_DIR)
while (control/'cli-hold').exists(): time.sleep(.02)
if (control/'cli-fail').exists(): raise SystemExit(3)
output=sys.argv[sys.argv.index('--export-3mf')+1]
if '--curr-bed-type' in sys.argv: bed=sys.argv[sys.argv.index('--curr-bed-type')+1]
else:
    with zipfile.ZipFile('project.3mf') as project: bed=json.loads(project.read('Metadata/project_settings.config'))['curr_bed_type']
with zipfile.ZipFile(ARTIFACT) as source, zipfile.ZipFile(output,'w',zipfile.ZIP_DEFLATED) as target:
    for entry in source.infolist():
        data=source.read(entry.filename)
        if entry.filename=='Metadata/project_settings.config':
            settings=json.loads(data)
            settings.update(printer_settings_id=profiles['printer']['name'],print_settings_id=profiles['process']['name'],filament_settings_id=[p['name'] for p in materials],curr_bed_type=bed)
            for key in ['filament_type','filament_colour','nozzle_temperature','nozzle_temperature_initial_layer']: settings[key]=[v for p in materials for v in (p.get(key,['#FFFFFF']) if key=='filament_colour' else p[key])]
            data=json.dumps(settings).encode()
        if entry.filename=='Metadata/slice_info.config':
            metadata=ET.fromstring(data)
            plate=metadata.find('plate')
            for old in plate.findall('filament'): plate.remove(old)
            for i,p in enumerate(materials): ET.SubElement(plate,'filament',id=str(i+1),type=p['filament_type'][0],color=p.get('filament_colour',['#FFFFFF'])[0],used_for_object='true' if i==0 else 'false')
            data=ET.tostring(metadata)
        target.writestr(entry.filename,data)
'''.replace('FIXTURE_CONTROL_DIR', repr(str(root))).replace('TRACE', repr(str(root/'cli.jsonl'))).replace('ARTIFACT', repr(str(REPO/'tests/fixtures/p1_print.gcode.3mf')))
    (app/'AppRun').write_text(script); (app/'AppRun').chmod(0o700)
    return app


class Rig:
    def __init__(self, binary, output, version='v3', appdir=None):
        self.binary = str(Path(binary).resolve()); self.output = Path(output).resolve(); self.output.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='orca-print-test-'); self.root = Path(self.temp.name)
        self.store = self.root/'plates'; self.store.mkdir()
        certificate(self.root, 'trusted', version)
        self.broker = PrintBroker(self.root/'trusted.pem', self.root/'trusted.key')
        self.ftp = Ftps(self.root/'trusted.pem', self.root/'trusted.key')
        self.files = {'parts/cube.stl': (REPO/'tests/fixtures/cube.stl').read_bytes()}
        rig = self
        class Scad(BaseHTTPRequestHandler):
            def do_GET(self):
                key = urllib.parse.unquote(urllib.parse.urlsplit(self.path).path.removeprefix('/models/'))
                data = json.dumps(sorted(rig.files)).encode() if self.path == '/api/models' else rig.files.get(key)
                self.send_response(200 if data is not None else 404); self.end_headers(); self.wfile.write(data or b'not found')
            def log_message(self, *_): pass
        self.scad = ThreadingHTTPServer(('127.0.0.1', 0), Scad)
        threading.Thread(target=self.scad.serve_forever, daemon=True).start()
        with socket.socket() as sock: sock.bind(('127.0.0.1', 0)); self.port = sock.getsockname()[1]
        self.base = f'http://127.0.0.1:{self.port}'
        self.env = {k:v for k,v in os.environ.items() if not k.startswith('P1_') and k not in ('ORCA_APPDIR','SCAD_LIVE_URL','DISCORD_WEBHOOK_URL','ORCA_PUBLIC_URL')}
        self.env.update(PORT=str(self.port), PLATES_DIR=str(self.store), ORCA_APPDIR=str(appdir or fake_slicer(self.root)),
            SCAD_LIVE_URL=f'http://127.0.0.1:{self.scad.server_port}', P1_IP='127.0.0.1', P1_SERIAL=SERIAL,
            P1_ACCESS_CODE=SECRET,P1_TLS_CERT=str(self.root/'trusted.pem'),P1_MQTT_PORT=str(self.broker.port),
            P1_FTPS_PORT=str(self.ftp.port),P1_START_TIMEOUT_SECS='10')
        self.full = json.loads((REPO/'tests/fixtures/p1_status.json').read_text())
        self.full['print']['ams']['tray_exist_bits'] = '9'
        self.full['print']['ams']['ams'][0]['tray'][3].update(tray_type='PLA', tray_color='00FFFFFF')
        self.process = None; self.log = (self.output/'server.log').open('a'); self.materials = []; self.plate = None

    def api(self, path='/api/queue?printer_id=p1', body=None, method=None, expected=200, origin=None):
        headers = {'Content-Type':'application/json'}
        if origin: headers['Origin'] = origin
        req = urllib.request.Request(self.base+path,data=None if body is None else json.dumps(body).encode(),headers=headers,method=method)
        try: response=urllib.request.urlopen(req, timeout=10)
        except urllib.error.HTTPError as e: response=e
        with response:
            raw=response.read(); assert SECRET.encode() not in raw
            if expected is not None: assert response.status==expected,(path,response.status,raw)
            value=(json.loads(raw) if 'application/json' in response.headers.get('Content-Type','') else raw.decode()) if raw else None
            return (response.status,value) if expected is None else value

    def launch(self):
        requests=len(self.broker.requests)
        self.process=subprocess.Popen([self.binary],cwd=self.root,env=self.env,stdout=self.log,stderr=self.log)
        until(lambda:urllib.request.urlopen(self.base+'/healthz',timeout=1).status==200,30)
        until(lambda:len(self.broker.requests)>requests)

    def stop(self, kill=False):
        if self.process and self.process.poll() is None:
            self.process.kill() if kill else self.process.terminate(); self.process.wait(timeout=10)
        self.process=None

    def idle(self):
        self.broker.send(self.full); until(lambda:self.api()['printer']['ready_to_print'])

    def seed(self):
        self.idle()
        for name,color in [('PLA 白','FFFFFFFF'),('PLA 青','00FFFFFF')]:
            f=self.api('/api/filaments',dict(name=name,vendor='Fixture',material='PLA',color=color,bambu_filament_id=None),expected=201)
            self.api(f'/api/filaments/{f["id"]}/settings',dict(machine_profile_key=MACHINE,base_profile_key=FILAMENT,overrides_json={'nozzle_temperature':215}),expected=201)
            self.materials.append(f)
        slots=self.api('/api/printers/p1/ams')['slots']
        for index,material in [(0,self.materials[0]),(3,self.materials[1])]:
            slot=next(s for s in slots if s['slot_index']==index)
            self.api(f'/api/printers/p1/ams/{slot["id"]}',dict(revision=slot['revision'],filament_id=material['id']),'PUT',204)
        self.plate=self.api('/api/plates/import',dict(name='最新のキューブ',models=[dict(name='parts/cube.stl',source='parts/cube.stl',quantity=2)]),expected=201)

    def specification(self, slot=3, material=None):
        selected=next(s for s in self.api('/api/printers/p1/ams')['slots'] if s['slot_index']==slot and s['ams_id']==0)
        return dict(ams_slot_id=selected['id'],filament_id=material or selected['filament_id'],required_machine_profile_key=MACHINE,process_profile_key=PROCESS,bed_type=BED)

    def command(self, action, base=None):
        base=self.api() if base is None else base
        return dict(epoch=base['epoch'],generation=base['generation'],request_id=base['request_id'],action=action)

    def send(self, action, expected=200):return self.api(body=self.command(action),expected=expected)
    def configure(self, specification=None, plate=None):
        plate=self.api('/api/plates/'+(plate or self.plate)['id'])
        conditions={**plate['conditions'], **{k:v for k,v in (specification or self.specification()).items() if k!='ams_slot_id'}}
        if plate['conditions']!=conditions:
            plate=self.api('/api/plates/'+plate['id'],dict(name=plate['name'],version=plate['version'],models=plate['models'],conditions=conditions),'PUT')
        if self.plate and plate['id']==self.plate['id']:self.plate=plate
        return plate
    def add_action(self, specification=None, plate=None):
        plate=self.configure(specification,plate)
        return dict(type='add',plate_id=plate['id'],plate_version=plate['version'])
    def add(self, slot=3):
        plate=self.plate
        if slot!=3:
            plate=self.api('/api/plates/import',dict(name=plate['name']+' white',models=[{k:v for k,v in m.items() if k!='id'} for m in plate['models']]),expected=201)
        return self.send(self.add_action(self.specification(slot),plate))['waiting'][-1]
    def next(self, job, expected=200):
        q=self.api()
        return self.api(body=self.command(dict(type='next',expected_job=job['id'],removed_job=q['current']['id'] if q['current'] else None,cleared=True),q),expected=expected)
    def report(self, state, index=-1):
        value=copy.deepcopy(self.full); command=self.broker.prints[index]
        value['print'].update(gcode_state=state,subtask_name=command['subtask_name'],gcode_file=command['file'])
        self.broker.send(value)
    def phase(self, name):return until(lambda:self.api()['current'] and self.api()['current']['state']==name)
    def traces(self):return [json.loads(line) for line in (self.root/'cli.jsonl').read_text().splitlines()]
    def close(self):
        self.stop();self.log.close();self.scad.shutdown();self.scad.server_close();self.ftp.close();self.broker.close()
        assert SECRET.encode() not in (self.output/'server.log').read_bytes()
        self.temp.cleanup()


def printer_control(rig):
    class Control(BaseHTTPRequestHandler):
        def do_GET(self): self.reply()
        def do_POST(self):
            value=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            if 'slice_hold' in value:
                path=rig.root/'cli-hold';path.touch() if value['slice_hold'] else path.unlink(missing_ok=True)
            elif 'slice_fail' in value:
                path=rig.root/'cli-fail';path.touch() if value['slice_fail'] else path.unlink(missing_ok=True)
            elif value.get('fail_upload'): rig.ftp.actions.put('fail')
            elif value.get('disconnect'): rig.broker.actions.put('disconnect')
            elif value.get('state'): rig.report(value['state'],value.get('index',-1))
            else: rig.broker.send(rig.full)
            self.reply()
        def reply(self):
            data=json.dumps(dict(prints=rig.broker.prints,uploads=rig.ftp.uploads,count=len(rig.broker.prints))).encode()
            self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
        def log_message(self,*_): pass
    control=ThreadingHTTPServer(('127.0.0.1',0),Control)
    threading.Thread(target=control.serve_forever,daemon=True).start()
    return control
