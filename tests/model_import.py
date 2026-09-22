"""Local-file import and slicing with own geometry; never contacts a real printer."""
import io
import json
import os
import struct
import sqlite3
import subprocess
import sys
import urllib.error
import urllib.request
import zipfile
from pathlib import Path
from print_fixture import Rig, REPO, until


def fixture(painted=False, multiple=True):
    # A 20 mm cube and a separate raised numeral stroke retain their 23 x 20 x 20 mm envelope.
    points = [(0,0,0),(20,0,0),(20,20,0),(0,20,0),(0,0,20),(20,0,20),(20,20,20),(0,20,20)]
    faces = [(0,2,1),(0,3,2),(4,5,6),(4,6,7),(0,1,5),(0,5,4),(1,2,6),(1,6,5),(2,3,7),(2,7,6),(3,0,4),(3,4,7)]
    vertices = ''.join(f'<vertex x="{x}" y="{y}" z="{z}"/>' for x,y,z in points)
    triangles = ''.join(f'<triangle v1="{a}" v2="{b}" v3="{c}"'+ (' paint_color="0C"' if painted and i==0 else '')+'/>' for i,(a,b,c) in enumerate(faces))
    core = 'http://schemas.microsoft.com/3dmanufacturing/core/2015/02'
    production = 'http://schemas.microsoft.com/3dmanufacturing/production/2015/06'
    part = f'<model unit="millimeter" xmlns="{core}"><resources><object id="1"><mesh><vertices>{vertices}</vertices><triangles>{triangles}</triangles></mesh></object></resources></model>'
    components = f'<component p:path="/3D/Objects/part.model" objectid="1"/><component p:path="/3D/Objects/part.model" objectid="1" transform="0.05 0 0 0 0.25 0 0 0 0.05 22 1 0"/>'
    build = '<item objectid="10"/>' + ('<item objectid="10" transform="1 0 0 0 1 0 0 0 1 300 0 0"/>' if multiple else '')
    model = f'<model unit="millimeter" xmlns="{core}" xmlns:p="{production}" requiredextensions="p"><resources><object id="10"><components>{components}</components></object></resources><build>{build}</build></model>'
    plates = ''.join(f'<plate><metadata key="plater_id" value="{i+1}"/><metadata key="plater_name" value="寸法確認 {i+1}"/><model_instance><metadata key="object_id" value="10"/><metadata key="instance_id" value="{i}"/></model_instance></plate>' for i in range(2 if multiple else 1))
    stream = io.BytesIO()
    with zipfile.ZipFile(stream,'w',zipfile.ZIP_DEFLATED) as archive:
        for name, data in {
            '[Content_Types].xml':'<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>',
            '_rels/.rels':'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="start" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" Target="/3D/3dmodel.model"/></Relationships>',
            '3D/3dmodel.model':model, '3D/Objects/part.model':part,
            'Metadata/model_settings.config':'<config>'+plates+'</config>',
            'Metadata/uninterpreted-extension.xml':'<vendor material="preserve exactly"/>',
            'Metadata/plate_1.gcode':'UNTRUSTED SOURCE GCODE - MUST NOT RUN',
        }.items():
            archive.writestr(name,data)
    return stream.getvalue()


def upload(base, path, files, fields=(), expected=200):
    body = bytearray()
    for key,value in fields:
        body.extend(f'--orca\r\nContent-Disposition: form-data; name="{key}"\r\n\r\n{value}\r\n'.encode())
    for name,data in files:
        body.extend(f'--orca\r\nContent-Disposition: form-data; name="models"; filename="{name}"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode());body.extend(data);body.extend(b'\r\n')
    body.extend(b'--orca--\r\n')
    request = urllib.request.Request(base+path,bytes(body),{'Content-Type':'multipart/form-data; boundary=orca'})
    try: response=urllib.request.urlopen(request,timeout=120)
    except urllib.error.HTTPError as error: response=error
    with response:
        raw=response.read();assert response.status==expected,(response.status,raw)
        return json.loads(raw) if 'application/json' in response.headers.get('Content-Type','') else raw


def run(binary, output, appdir=None):
    rig=Rig(binary,output,appdir=appdir)
    try:
        rig.launch();rig.seed();rig.stop();rig.env.pop('SCAD_LIVE_URL');rig.launch();rig.idle()
        fixture_dir=rig.output/'fixtures';fixture_dir.mkdir(exist_ok=True)
        single,painted,multi=fixture(multiple=False),fixture(painted=True),fixture()
        for name,data in [('single.3mf',single),('painted.3mf',painted),('multiple.3mf',multi)]: (fixture_dir/name).write_bytes(data)
        (fixture_dir/'cube.stl').write_bytes((REPO/'tests/fixtures/cube.stl').read_bytes())
        (fixture_dir/'bad.3mf').write_bytes(b'broken zip')
        files=[('multiple.3mf',multi)]
        info=upload(rig.base,'/api/plates/file-info',files);assert len(info)==2 and info[1]['print_reason'] is None
        derived=upload(rig.base,'/api/plates/file-preview',files,[('plate','1')])
        count=struct.unpack_from('<I',derived,80)[0];assert count==24
        points=[struct.unpack_from('<fff',derived,84+i*50+12+j*12) for i in range(count) for j in range(3)]
        low=[min(v[axis] for v in points) for axis in range(3)];high=[max(v[axis] for v in points) for axis in range(3)]
        assert low==[300,0,0] and high==[323,20,20],(low,high)
        p=upload(rig.base,'/api/plates/files',files,[('plate','1'),('name','Imported geometry'),('quantities','[2]')],201)
        assert p['conditions']==rig.api('/api/default-settings')['conditions']
        assert p['imported']['selection']['items'][0]['build_index']==1
        original=urllib.request.urlopen(rig.base+'/api/plates/'+p['id']+'/original').read();assert original==multi
        multicolor=upload(rig.base,'/api/plates/files',[('painted.3mf',painted)],[('plate','0'),('name','Preserved colors')],201)
        q=rig.api('/api/queue?printer_id=p1&plate_id='+multicolor['id']);assert not q['admission']['allowed'] and '多色' in q['admission']['reason']
        rig.send(dict(type='add',plate_id=multicolor['id'],plate_version=multicolor['version']),expected=409)
        before=rig.api('/api/plates')
        upload(rig.base,'/api/plates/files',[('bad.3mf',b'invalid')],[('name','Invalid')],400)
        assert rig.api('/api/plates')==before
        rig.stop();rig.launch();rig.idle()
        assert urllib.request.urlopen(rig.base+'/api/plates/'+multicolor['id']+'/original').read()==painted
        job=rig.send(dict(type='add',plate_id=p['id'],plate_version=p['version']))['waiting'][-1]
        def estimated():
            estimate=next(j for j in rig.api()['waiting'] if j['id']==job['id'])['estimate']
            assert not estimate or estimate['state']!='failed',estimate
            return estimate if estimate and estimate['state']=='ready' else None
        estimate=until(estimated,120)
        with sqlite3.connect(rig.store/'orca.sqlite3') as db:
            saved=json.loads(db.execute('SELECT estimate_json FROM print_jobs WHERE id=?',(job['id'],)).fetchone()[0])
        process=saved['input']['settings']['profiles']['process.json']
        assert process['enable_support']=='0' and process['brim_type']=='no_brim'
        assert b'UNTRUSTED SOURCE GCODE' not in json.dumps(saved).encode()
        if appdir:
            bundle=(rig.store/'jobs'/job['id']/('estimate-'+saved['id'])/'print.gcode.3mf').read_bytes()
            (rig.output/'resliced.gcode.3mf').write_bytes(bundle)
            with zipfile.ZipFile(io.BytesIO(bundle)) as archive:
                gcode=archive.read('Metadata/plate_1.gcode')
                assert b'UNTRUSTED SOURCE GCODE' not in gcode
            produced=upload(rig.base,'/api/plates/file-info',[('official.3mf',bundle)])
            assert len(produced)==1 and produced[0]['print_reason'] is None
            roundtrip=upload(rig.base,'/api/plates/file-preview',[('official.3mf',bundle)])
            assert struct.unpack_from('<I',roundtrip,80)[0]==48
            (rig.output/'resliced-preview.stl').write_bytes(roundtrip)
        if os.environ.get('IMPORT_BROWSER'):
            env=dict(os.environ,E2E_BASE_URL=rig.base,E2E_IMPORT_CONTEXT=json.dumps(dict(fixtures=str(fixture_dir))),E2E_EVIDENCE_DIR=str(rig.output/'browser'))
            subprocess.run(['npm','run','test:e2e'],cwd=REPO/'client',env=env,check=True)
        assert not rig.broker.prints and not rig.ftp.uploads
        (rig.output/'result.json').write_text(json.dumps(dict(real_orca=bool(appdir),selected_bounds=[low,high],faces=count,exact_original=True,multicolor_preserved_and_blocked=True,no_scad=True,estimate=estimate,printer_commands=0,uploads=0),ensure_ascii=False,indent=2))
    finally: rig.close()

if __name__=='__main__': run(sys.argv[1],sys.argv[2],sys.argv[3] if len(sys.argv)>3 else None)
