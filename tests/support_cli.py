"""Official Orca 2.4.2 support matrix; all uploads/starts go only to disposable printer peers.

Run with IMAGE OUTPUT, or BINARY OUTPUT APPDIR for an installed official AppDir.
"""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
import zipfile
from container import ContainerRig, docker
from print_fixture import Rig, MACHINE, PROCESS, REPO, until


def inspect(path, enabled, distinct, unused, temperatures, bed):
    with zipfile.ZipFile(path) as z:
        settings=json.loads(z.read('Metadata/project_settings.config'))
        xml=ET.fromstring(z.read('Metadata/slice_info.config'))
        lines=z.read('Metadata/plate_1.gcode').decode().splitlines()
    assert settings['enable_support']==str(int(enabled))
    assert settings['enforce_support_layers']=='0'
    expected=2 if distinct else 1
    assert len(settings['filament_settings_id'])==len(settings['filament_type'])==expected
    assert settings['textured_plate_temp']==[str(bed[1])]*expected
    assert settings['textured_plate_temp_initial_layer']==[str(bed[0])]*expected
    # These bundled profiles have two PLA variants and one Generic PETG variant.
    # Verify each variant, then independently inspect the actual tool-change temperatures below.
    assert settings['nozzle_temperature']==[str(t[1]) for t in temperatures for _ in range(t[2])]
    assert settings['nozzle_temperature_initial_layer']==[str(t[0]) for t in temperatures for _ in range(t[2])]
    if enabled:
        assert settings['support_type'] in ['normal(auto)','tree(auto)']
        assert settings['support_filament']=='1' and settings['support_interface_filament']==str(expected)
        assert settings['support_interface_not_for_body']=='1'
        assert int(settings['support_interface_top_layers'])>0
        assert float(settings['support_top_z_distance'])>0 and float(settings['support_interface_spacing'])>0
    if distinct:
        assert settings['enable_prime_tower']=='1'
        assert all(settings[k]=='0' for k in ['flush_into_infill','flush_into_objects','flush_into_support'])
        assert all(float(settings['flush_volumes_matrix'][i])>0 for i in [1,2])
        assert all(float(v)>0 for v in settings['flush_multiplier'])
    tool=0;feature='';extrusion={};flush=False;purge=0;switches=[];bed_commands=[];nozzle_commands=[]
    for line in lines:
        if line.startswith('; FEATURE: '):feature=line[11:]
        if line.strip()=='; FLUSH_START':flush=True
        if line.strip()=='; FLUSH_END':flush=False
        command=line.split(';',1)[0].strip()
        change=re.fullmatch(r'T([01])',command)
        if change:
            tool=int(change[1]);switches.append(tool)
        e=re.search(r'\bE(-?[\d.]+)',command)
        if re.match(r'G[0123] ',command) and e and float(e[1])>0:
            if flush:purge+=float(e[1])
            if re.search(r'\b[XY]-?[\d.]',command):extrusion[(feature,tool)]=extrusion.get((feature,tool),0)+float(e[1])
        if command.startswith(('M140 ', 'M190 ')):
            value=re.search(r'\bS([\d.]+)',command);assert value,command
            bed_commands.append(float(value[1]))
        if command.startswith(('M104 ', 'M109 ')):
            value=re.search(r'\bS([\d.]+)',command)
            if value:nozzle_commands.append(float(value[1]))
    assert bed_commands and set(bed_commands)<=set([0,*bed])
    active={i for (kind,i),amount in extrusion.items() if kind=='Support interface' and amount>0}
    body={i for (kind,i),amount in extrusion.items() if kind=='Support' and amount>0}
    assert all(i==0 for (kind,i) in extrusion if kind in ['Inner wall','Outer wall','Sparse infill','Internal solid infill','Bottom surface','Top surface'])
    if enabled and not unused:
        assert active=={1 if distinct else 0} and body=={0},(active,body)
    else:assert not active and not body,(active,body)
    if distinct and not unused:
        assert 0 in switches and 1 in switches and purge>0
        assert any(kind=='Prime tower' and amount>0 for (kind,_),amount in extrusion.items())
        assert all(t[1] in nozzle_commands for t in temperatures)
    used=xml.findall('plate/filament')
    assert [int(f.attrib['id']) for f in used]==([1,2] if distinct and not unused else [1])
    seconds=int(xml.find("plate/metadata[@key='prediction']").attrib['value'])
    return dict(seconds=seconds,types=settings['filament_type'],colors=settings['filament_colour'],support_body_indices=sorted(body),interface_indices=sorted(active),switches=switches,purge_filament_mm=purge,bed_commands=bed_commands,nozzle_commands=sorted(set(nozzle_commands)))


def run(target, output, appdir=None):
    rig=Rig(target,output,appdir=appdir) if appdir else ContainerRig(target,output)
    def archive(job, name):
        output=rig.output/name
        if appdir:
            paths=list((rig.store/'jobs'/job['id']).glob('estimate-*/print.gcode.3mf'))
            assert len(paths)==1,paths;output.write_bytes(paths[0].read_bytes())
        else:
            paths=docker('exec',rig.name,'find','/data/plates/jobs/'+job['id'],'-name','print.gcode.3mf').splitlines()
            assert len(paths)==1,paths;docker('cp',rig.name+':'+paths[0],str(output))
        return output
    def setting(f, base, temps, bed):
        data=dict(machine_profile_key=MACHINE,base_profile_key=base,overrides_json=dict(nozzle_temperature_initial_layer=temps[0],nozzle_temperature=temps[1],bed_temperature_initial_layer=bed[0],bed_temperature=bed[1]))
        old=rig.api('/api/filaments/'+f['id'])['settings']
        path='/api/filaments/'+f['id']+'/settings'+('/'+old[0]['id'] if old else '')
        rig.api(path,data,'PUT' if old else 'POST',200 if old else 201)
    results=[]
    try:
        rig.files['parts/support-cantilever.stl']=(REPO/'tests/fixtures/support-cantilever.stl').read_bytes()
        rig.full['print']['ams']['tray_exist_bits']='b'
        rig.full['print']['ams']['ams'][0]['tray'][1].update(tray_type='PETG',tray_color='FFFFFFFF')
        rig.launch();rig.seed()
        assert rig.api('/api/slicer/profiles')['version']=='2.4.2'
        pla,blue=rig.materials
        petg=rig.api('/api/filaments',dict(name='PETG 白',vendor='Fixture',material='PETG',color='FFFFFFFF',bambu_filament_id=None),expected=201)
        temps=[(220,215,2),(230,225,2),(250,255,1)];beds=[(60,55),(45,40),(75,70)]
        for f,base,t,b in zip([pla,blue,petg],['Generic PLA High Speed @BBL X1C','Generic PLA High Speed @BBL X1C','Generic PETG'],temps,beds):setting(f,base,t,b)
        slot=next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==1)
        rig.api('/api/printers/p1/ams/'+slot['id'],dict(revision=slot['revision'],filament_id=petg['id']),'PUT',204)
        before_settings=rig.api('/api/filaments/'+petg['id'])['settings']
        for name,on,main,interface,unused in [('off',False,0,0,False),('same',True,0,0,False),('pla-colors',True,0,1,False),('pla-petg',True,0,2,False),('petg-pla',True,2,0,False),('unused',True,0,2,True)]:
            filaments=[pla,blue,petg];distinct=on and main!=interface
            source='parts/cube.stl' if unused else 'parts/support-cantilever.stl'
            conditions=dict(required_machine_profile_key=MACHINE,filament_id=filaments[main]['id'],process_profile_key=PROCESS,bed_type='Textured PEI Plate',support_enabled=on,support_interface_filament_id=filaments[interface]['id'])
            rig.plate=rig.api('/api/plates/import',dict(name=name,models=[dict(name=source,source=source,quantity=1)],conditions=conditions),expected=201)
            job=rig.send(dict(type='add',plate_id=rig.plate['id'],plate_version=rig.plate['version']))['waiting'][0]
            def ready():
                e=rig.api()['waiting'][0]['estimate'];assert e['state']!='failed',e;return e['state']=='ready'
            until(ready,120)
            path=archive(job,name+'.gcode.3mf')
            result=inspect(path,on,distinct,unused,[temps[main]]+([temps[interface]] if distinct else []),beds[main])
            assert result['seconds']==rig.api()['waiting'][0]['estimate']['seconds']
            count=len(rig.broker.prints);rig.next(job);until(lambda:len(rig.broker.prints)==count+1,120)
            assert rig.ftp.contents[-1]==path.read_bytes(), 'Preparation must preserve the actual estimated G-code bytes'
            mapping=[ [0,3,1][main] ]+([ [0,3,1][interface] ] if distinct else [])
            assert rig.broker.prints[-1]['ams_mapping']==mapping
            assert rig.api()['current']['estimate']['seconds']==result['seconds']
            results.append(dict(case=name,mapping=mapping,**result));print(json.dumps(results[-1]),flush=True)
            rig.report('RUNNING');rig.phase('printing');rig.report('FINISH');rig.phase('awaiting_removal')
            rig.send(dict(type='discard',expected_job=job['id'],cleared=True));rig.idle()
        assert rig.api('/api/filaments/'+petg['id'])['settings']==before_settings
        result=dict(target=target,official_cli='2.4.2',cases=results,unchanged_common_settings=True,isolated_print_commands=6,real_printer_commands=0)
        (rig.output/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
    finally:
        if not appdir:
            with (rig.output/'server.log').open('a') as log:subprocess.run(['docker','logs',rig.name],stdout=log,stderr=log)
            rig.stop();docker('run','--rm','--user','0','--entrypoint','chown','-v',str(rig.store)+':/data/plates',target,'-R',f'{os.getuid()}:{os.getgid()}','/data/plates')
        rig.close()


if __name__=='__main__':run(*sys.argv[1:])
