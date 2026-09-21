"""Real OrcaSlicer integration: python3 tests/slicer_cli.py APPDIR OUTPUT_DIR."""
import io
import json
import os
from pathlib import Path
import struct
import sys
import urllib.error
import urllib.request
import urllib.parse
import xml.etree.ElementTree as ET
import zipfile

REPO = Path(__file__).resolve().parents[1]
CORE = '{http://schemas.microsoft.com/3dmanufacturing/core/2015/02}'
PRODUCTION = '{http://schemas.microsoft.com/3dmanufacturing/production/2015/06}'


def boxes(data):
    """Compose component/build transforms before comparing world coordinates."""
    archive = zipfile.ZipFile(io.BytesIO(data))

    def transform(vertex, text):
        matrix = list(map(float, text.split())) if text else [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]
        return [sum(vertex[j] * matrix[j * 3 + i] for j in range(3)) + matrix[9 + i] for i in range(3)]

    def vertices(path, object_id):
        root = ET.fromstring(archive.read(path))
        obj = root.find(f'{CORE}resources/{CORE}object[@id="{object_id}"]')
        result = [[float(v.get(a)) for a in ('x', 'y', 'z')] for v in obj.findall(f'{CORE}mesh/{CORE}vertices/{CORE}vertex')]
        for component in obj.findall(f'{CORE}components/{CORE}component'):
            target = component.get(PRODUCTION + 'path', path).lstrip('/')
            result.extend(transform(v, component.get('transform')) for v in vertices(target, component.get('objectid')))
        return result

    root = ET.fromstring(archive.read('3D/3dmodel.model'))
    result = []
    for item in root.findall(f'{CORE}build/{CORE}item'):
        points = [transform(v, item.get('transform')) for v in vertices('3D/3dmodel.model', item.get('objectid'))]
        result.append([[min(v[a] for v in points), max(v[a] for v in points)] for a in range(3)])
    return sorted(result)


def main():
    from print_fixture import Rig, MACHINE, FILAMENT, PROCESS
    from printer_mqtt import until
    appdir=Path(sys.argv[1]).resolve();output=Path(sys.argv[2]).resolve();output.mkdir(parents=True,exist_ok=True)
    binary=os.environ.get('ORCA_TEST_BINARY',str(REPO/'target/debug/orca-server'))
    rig=Rig(binary,output,appdir=appdir);results={}
    try:
        rig.launch();rig.seed()
        for index,machine in enumerate([MACHINE,'Bambu Lab A1 mini 0.2 nozzle']):
            selection=rig.api('/api/slicer/profiles?machine='+urllib.parse.quote(machine))['defaults']
            if index:
                settings={k:v for k,v in rig.api('/api/printers/p1').items() if k not in ('id','status','machine','configuration_error')}
                settings.update(machine_profile_key=machine,default_process_profile_key=selection['process'])
                requests=len(rig.broker.requests)
                rig.api('/api/printers/p1',settings,'PUT');until(lambda:len(rig.broker.requests)>requests);rig.idle()
                choices=rig.api(f'/api/filaments/{rig.materials[1]["id"]}/profiles?machine='+urllib.parse.quote(machine))
                base=next(p['key'] for p in choices if p['key'].startswith('Generic PLA'))
                rig.api(f'/api/filaments/{rig.materials[1]["id"]}/settings',dict(machine_profile_key=machine,base_profile_key=base,overrides_json={'nozzle_temperature':215}),expected=201)
            spec=rig.specification();spec.update(required_machine_profile_key=machine,process_profile_key=selection['process'])
            job=rig.send(rig.add_action(spec))['waiting'][-1]
            rig.next(job);until(lambda:len(rig.broker.prints)==index+1,90)
            current=rig.api()['current'];directory=rig.store/current['artifact_path']
            project=(directory/'project.3mf').read_bytes();printed=(directory/'print.gcode.3mf').read_bytes()
            assert rig.ftp.contents[-1]==printed
            before,after=boxes(project),boxes(printed);assert len(before)==len(after)==2
            size=256 if index==0 else 180
            assert all(0<=low<high<=limit for box in before for (low,high),limit in zip(box,[size,size,250 if index==0 else 180]))
            assert any(before[0][axis][1]<=before[1][axis][0] or before[1][axis][1]<=before[0][axis][0] for axis in (0,1)),before
            assert all(abs(x-y)<.001 for a,b in zip(before,after) for c,d in zip(a,b) for x,y in zip(c,d))
            with zipfile.ZipFile(io.BytesIO(printed)) as archive:
                settings=json.loads(archive.read('Metadata/project_settings.config'))
                gcode=archive.read('Metadata/plate_1.gcode')
                assert len(gcode)>1000 and b'215' in gcode
                assert settings['printer_settings_id']==machine
                assert settings['nozzle_diameter']==['0.4' if index==0 else '0.2']
                assert settings['nozzle_temperature'][0]=='215',settings['nozzle_temperature']
            assert not any(n.endswith('.gcode') for n in zipfile.ZipFile(io.BytesIO(project)).namelist())
            (output/f'{index}-project.3mf').write_bytes(project);(output/f'{index}-print.gcode.3mf').write_bytes(printed)
            rig.report('RUNNING');rig.phase('printing');rig.report('FINISH');rig.phase('awaiting_removal')
            rig.send(dict(type='discard',expected_job=job['id'],cleared=True))
        results.update(real_cli_version='2.4.2',references_resolved_at_start=True,quantity_arranged_without_overlap=True,registered_material_override_applied=True,p1_04_and_a1mini_02_profiles_preserved=True,uploaded_exact_execution_artifact=True)
        # A plate that cannot fit must never send a start command or consume the waiting successor.
        cube=bytearray(rig.files['parts/cube.stl'])
        for triangle in range(struct.unpack_from('<I',cube,80)[0]):
            for offset in range(96+triangle*50,132+triangle*50,4):
                struct.pack_into('<f',cube,offset,struct.unpack_from('<f',cube,offset)[0]*20)
        rig.files['parts/cube.stl']=bytes(cube)
        job=rig.send(rig.add_action(spec))['waiting'][-1]
        rig.next(job);rig.phase('needs_attention');assert len(rig.broker.prints)==2
        assert rig.api('/api/plates/'+rig.plate['id'])==rig.plate
        results['impossible_layout_preserves_composition_without_start']=True
    finally:rig.close()
    (output/'result.json').write_text(json.dumps(results,indent=2));print(json.dumps(results))


if __name__=='__main__':main()
