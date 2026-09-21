"""Official Orca estimates: python3 tests/estimate_cli.py BINARY APPDIR OUTPUT_DIR."""
import json
from pathlib import Path
import re
import sqlite3
import sys
import xml.etree.ElementTree as ET
import zipfile
from print_fixture import Rig, PROCESS
from printer_mqtt import until


def run(binary, appdir, output):
    rig=Rig(binary,output,appdir=Path(appdir).resolve()); results=[]
    def record(job):
        with sqlite3.connect(rig.store/'orca.sqlite3') as c:return json.loads(c.execute('SELECT estimate_json FROM print_jobs WHERE id=?',(job['id'],)).fetchone()[0])
    try:
        rig.launch();rig.seed()
        for quantity,process in [(1,PROCESS),(2,PROCESS),(2,'0.16mm Optimal @BBL X1C')]:
            plate=rig.configure();plate['conditions']['process_profile_key']=process
            plate['models'][0]['quantity']=quantity
            rig.plate=rig.api('/api/plates/'+plate.pop('id'),plate,'PUT')
            job=rig.send(dict(type='add',plate_id=rig.plate['id'],plate_version=rig.plate['version']))['waiting'][0]
            def finished():
                current=rig.api()['waiting'][0]
                assert current['estimate']['state']!='failed',current['estimate']
                return current['estimate']['state']=='ready'
            until(finished,120)
            current=rig.api()['waiting'][0];snapshot=record(job)
            directory=rig.store/'jobs'/job['id']/('estimate-'+snapshot['id'])
            printed=(directory/'print.gcode.3mf').read_bytes()
            with zipfile.ZipFile(directory/'print.gcode.3mf') as archive:
                prediction=int(ET.fromstring(archive.read('Metadata/slice_info.config')).find("plate/metadata[@key='prediction']").attrib['value'])
                settings=json.loads(archive.read('Metadata/project_settings.config'))
                gcode=archive.read('Metadata/plate_1.gcode').decode()
            total=re.search(r'total estimated time: ([^\r\n]+)',gcode)[1]
            seconds=sum(int(n)*{'d':86400,'h':3600,'m':60,'s':1}[unit] for n,unit in re.findall(r'(\d+)([dhms])',total))
            assert prediction==current['estimate']['seconds'] and abs(prediction-seconds)<=1
            assert settings['print_settings_id']==process
            assert len(list(directory.glob('*.stl')))==quantity
            assert not rig.broker.prints and not rig.ftp.uploads and rig.api()['current'] is None
            (rig.output/f'case-{len(results)}.gcode.3mf').write_bytes(printed)
            results.append(dict(quantity=quantity,process=process,seconds=prediction,gcode_total=total,field='Metadata/slice_info.config:plate/metadata[prediction]',unit='seconds'))
            if len(results)<3:rig.send(dict(type='remove',job_id=job['id']))
        assert results[1]['seconds']>results[0]['seconds'] and results[2]['seconds']!=results[1]['seconds']
        rig.stop();rig.launch();rig.idle();assert record(job)==snapshot
        before=(rig.output/'server.log').read_text().count('OrcaSlicer exited')
        rig.next(job);until(lambda:len(rig.broker.prints)==1,120)
        assert rig.ftp.contents[0]==printed
        assert (rig.output/'server.log').read_text().count('OrcaSlicer exited')==before
        assert rig.api()['current']['estimate']['seconds']==results[-1]['seconds']
        (rig.output/'result.json').write_text(json.dumps(dict(official_cli='2.4.2',cases=results,estimate_sends_no_commands=True,restart_preserves_estimate=True,same_input_reuses_exact_artifact=True,print_commands=1,uploads=1),indent=2));print(json.dumps(results))
    finally:rig.close()


if __name__=='__main__':run(*sys.argv[1:])
