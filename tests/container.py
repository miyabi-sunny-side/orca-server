"""Verify a Linux image with isolated protocol peers: python3 tests/container.py IMAGE OUTPUT_DIR."""
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid
import zipfile
import xml.etree.ElementTree as ET
from print_fixture import Rig, REPO, until
from slicer_cli import boxes


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()


class ContainerRig(Rig):
    def __init__(self, image, output):
        super().__init__('/unused', output)
        self.image=image; self.name='orca-check-'+uuid.uuid4().hex
        # This bind directory belongs only to this test, never an existing installation.
        docker('run','--rm','--user','0','--entrypoint','chown','-v',str(self.store)+':/data/plates',image,'10001:10001','/data/plates')
    def launch(self):
        before=len(self.broker.requests)
        args=['run','-d','--name',self.name,'--network','host','-v',str(self.store)+':/data/plates',
            '-v',str(self.root/'trusted.pem')+':/config/cert.pem:ro']
        for key in ['PORT','P1_IP','P1_SERIAL','P1_ACCESS_CODE','P1_MQTT_PORT','P1_FTPS_PORT','P1_START_TIMEOUT_SECS','SCAD_LIVE_URL']:
            args+=['-e',key+'='+self.env[key]]
        args+=['-e','P1_TLS_CERT=/config/cert.pem',self.image]
        docker(*args)
        until(lambda: self.api('/api/plates') is not None,30)
        until(lambda:len(self.broker.requests)>before)
    def stop(self, kill=False):
        subprocess.run(['docker','rm','-f',self.name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)


def main():
    rig=ContainerRig(sys.argv[1],sys.argv[2])
    try:
        rig.launch();rig.seed(); reference=rig.plate
        material = rig.api('/api/filaments/'+rig.materials[1]['id'])['settings'][0]
        setting = {k:material[k] for k in ['machine_profile_key','base_profile_key','overrides_json']}
        setting['overrides_json'].update(bed_temperature_initial_layer=65,bed_temperature=65)
        rig.api('/api/filaments/'+rig.materials[1]['id']+'/settings/'+material['id'],setting,'PUT')
        assert docker('exec',rig.name,'id','-u')=='10001'
        assert not any(e.startswith(('DISPLAY=','WAYLAND_DISPLAY=')) for e in docker('exec',rig.name,'env').splitlines())
        assert rig.api('/api/slicer/profiles')['version']=='2.4.2'
        import urllib.request
        assert b'<!doctype html' in urllib.request.urlopen(rig.base).read().lower()
        cube=(REPO/'tests/fixtures/cube.stl').read_bytes()
        fields=[('name',None,b'Two uploaded cubes'),('models','a.stl',cube),('models','b.stl',cube)]
        body=b''
        for field,filename,value in fields:
            header=f'Content-Disposition: form-data; name="{field}"'
            if filename:header+=f'; filename="{filename}"'
            body+=b'--orca-boundary\r\n'+header.encode()+b'\r\n\r\n'+value+b'\r\n'
        body+=b'--orca-boundary--\r\n'
        req=urllib.request.Request(rig.base+'/api/plates',data=body,headers={'Content-Type':'multipart/form-data; boundary=orca-boundary'})
        rig.plate=json.loads(urllib.request.urlopen(req).read())
        job=rig.add(3); plate=rig.plate
        until(lambda:rig.api()['waiting'][0]['estimate']['state']=='ready',120)
        estimate=rig.api()['waiting'][0]['estimate']['seconds']
        assert estimate>0 and not rig.broker.prints and not rig.ftp.uploads
        waiting=rig.send(rig.add_action(rig.specification(0), reference))['waiting'][-1]
        rig.next(job)
        until(lambda:len(rig.broker.prints)==1,90)
        current=rig.api()['current']; artifact=Path('/data/plates')/current['artifact_path']
        data={}
        for key,filename in [('project','project.3mf'),('print','print.gcode.3mf')]:
            dest=rig.output/filename
            docker('cp',rig.name+':'+str(artifact/filename),str(dest));data[key]=dest.read_bytes()
        assert rig.ftp.contents[0]==data['print']
        bounds,printed=boxes(data['project']),boxes(data['print'])
        assert len(bounds)==len(printed)==2
        assert all(0<=lo<hi<=limit for box in bounds for (lo,hi),limit in zip(box,[256,256,250]))
        assert any(bounds[0][a][1]<=bounds[1][a][0] or bounds[1][a][1]<=bounds[0][a][0] for a in (0,1))
        assert all(abs(x-y)<.001 for a,b in zip(bounds,printed) for c,d in zip(a,b) for x,y in zip(c,d))
        archive=zipfile.ZipFile(io.BytesIO(data['print']))
        prediction=int(ET.fromstring(archive.read('Metadata/slice_info.config')).find("plate/metadata[@key='prediction']").attrib['value'])
        assert estimate==prediction==rig.api()['current']['estimate']['seconds']
        gcode=archive.read('Metadata/plate_1.gcode')
        assert len(gcode)>1000
        settings=json.loads(archive.read('Metadata/project_settings.config'))
        assert settings['textured_plate_temp_initial_layer']==['65'] and settings['textured_plate_temp']==['65']
        bed_commands=[line.split(';',1)[0].strip() for line in gcode.decode().splitlines() if line.startswith(('M140 ', 'M190 '))]
        assert any('S65' in line.split() for line in bed_commands), bed_commands
        assert all(any(value in line.split() for value in ['S65','S0']) for line in bed_commands), bed_commands

        (rig.output/'first.log').write_text(docker('logs',rig.name))
        rig.stop();rig.launch()
        q=rig.api();assert q['current']['id']==job['id'] and q['current']['state']=='needs_attention'
        assert [j['id'] for j in q['waiting']]==[waiting['id']]
        assert not q['allowed']['next']
        assert rig.api('/api/plates/'+plate['id'])==plate
        for model in plate['models']:
            assert urllib.request.urlopen(rig.base+f'/api/plates/{plate["id"]}/files/{model["id"]}').read()==cube
        rig.idle();assert len(rig.broker.prints)==1
        until(lambda:docker('inspect','--format','{{.State.Health.Status}}',rig.name)=='healthy',45)
        result=dict(image=rig.image,uid=10001,version='2.4.2',headless=True,estimated_seconds=estimate,estimate_before_start=True,world_bounds=bounds,persistent_composition_uploads_queue=True,uncertain_restart_no_resend=True,bed_temperature_override_gcode=65,healthy=True)
        (rig.output/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
    finally:
        try:(rig.output/'server.log').write_text(docker('logs',rig.name))
        finally:
            rig.stop()
            docker('run','--rm','--user','0','--entrypoint','chown','-v',str(rig.store)+':/data/plates',rig.image,'-R',f'{os.getuid()}:{os.getgid()}','/data/plates')
            rig.close()


if __name__=='__main__':main()
