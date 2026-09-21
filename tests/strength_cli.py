"""Measure actual toolpaths from the pinned Orca CLI; no printer commands are sent."""
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import re
import sqlite3
import statistics
import sys
import zipfile
from print_fixture import Rig, until


def run(binary, appdir, output):
    rig=Rig(binary,output,appdir=Path(appdir).resolve());results=[]
    try:
        rig.launch();rig.seed()
        cases=[('adaptivecubic',15,2),('gyroid',15,2),('adaptivecubic',30,2),('adaptivecubic',15,4),('adaptivecubic',0,2),('adaptivecubic',100,2)]
        for pattern,density,walls in cases:
            plate=rig.configure();plate['models'][0]['quantity']=1
            plate['conditions'].update(sparse_infill_pattern=pattern,sparse_infill_density=density,wall_loops=walls)
            rig.plate=rig.api('/api/plates/'+plate.pop('id'),plate,'PUT')
            job=rig.send(dict(type='add',plate_id=rig.plate['id'],plate_version=rig.plate['version']))['waiting'][0]
            def finished():
                estimate=rig.api()['waiting'][0]['estimate'];assert estimate['state']!='failed',estimate
                return estimate['state']=='ready'
            until(finished,120)
            with sqlite3.connect(rig.store/'orca.sqlite3') as c:
                record=json.loads(c.execute('SELECT estimate_json FROM print_jobs WHERE id=?',(job['id'],)).fetchone()[0])
            directory=rig.store/'jobs'/job['id']/('estimate-'+record['id'])
            archive=directory/'print.gcode.3mf'
            with zipfile.ZipFile(archive) as z:
                settings=json.loads(z.read('Metadata/project_settings.config'))
                gcode=z.read('Metadata/plate_1.gcode').decode()
            assert settings['sparse_infill_pattern']==pattern
            assert float(settings['sparse_infill_density'].rstrip('%'))==density
            for key,value in [('wall_loops',walls),('top_shell_layers',5*walls//2),('bottom_shell_layers',3*walls//2),('top_shell_thickness',walls/2),('bottom_shell_thickness',0)]:
                assert float(settings[key])==value,(key,settings[key],value)
            # Orca emits relative E. Count deposited filament only during XY moves, by actual feature and layer.
            assert 'M83' in gcode
            layers=[];sparse_paths=[]
            for block in gcode.split('; CHANGE_LAYER')[1:]:
                amounts=defaultdict(float);feature='';paths=[]
                for line in block.splitlines():
                    if line.startswith('; FEATURE: '):feature=line.removeprefix('; FEATURE: ')
                    if re.match(r'G[123] ',line) and re.search(r' [XY]-?[\d.]',line):
                        e=re.search(r' E(-?[\d.]+)',line)
                        if e and float(e[1])>0:
                            amounts[feature]+=float(e[1])
                            if feature=='Sparse infill':paths.append(line)
                layers.append(dict(amounts));sparse_paths+=paths
            sparse=[i for i,layer in enumerate(layers) if layer.get('Sparse infill',0)>0]
            middle=layers[10:-10]
            result=dict(pattern=pattern,density=density,walls=walls,layers=len(layers),sparse_layers=sparse,
                middle_inner_filament=statistics.median(layer.get('Inner wall',0) for layer in middle),
                middle_sparse_filament=statistics.median(layer.get('Sparse infill',0) for layer in middle),
                middle_solid_filament=statistics.median(layer.get('Internal solid infill',0) for layer in middle),
                sparse_path_hash=hashlib.sha256('\n'.join(sparse_paths).encode()).hexdigest(),
                layer_extrusion=layers,seconds=rig.api()['waiting'][0]['estimate']['seconds'])
            assert all(layer.get('Outer wall',0)>0 for layer in middle)
            if density in (0,100):assert not sparse
            else:assert len(sparse)>60 and result['middle_sparse_filament']>0
            (rig.output/f'case-{len(results)}.gcode.3mf').write_bytes(archive.read_bytes())
            (rig.output/f'case-{len(results)}-process.json').write_text((directory/'process.json').read_text())
            results.append(result)
            (rig.output/'result.json').write_text(json.dumps(results,indent=2))
            rig.send(dict(type='remove',job_id=job['id']))
        default,gyroid,denser,thicker,empty,solid=results
        assert default['sparse_path_hash']!=gyroid['sparse_path_hash']
        assert denser['middle_sparse_filament']>default['middle_sparse_filament']*1.3
        assert thicker['middle_inner_filament']>default['middle_inner_filament']*2.5
        assert thicker['sparse_layers'][0]>default['sparse_layers'][0]
        assert thicker['sparse_layers'][-1]<default['sparse_layers'][-1]
        assert empty['middle_solid_filament']==0 and solid['middle_solid_filament']>0
        assert not rig.broker.prints and not rig.ftp.uploads
        print(json.dumps([{k:v for k,v in r.items() if k not in ('layer_extrusion','sparse_layers')} for r in results]))
    finally:rig.close()


if __name__=='__main__':run(*sys.argv[1:])
