"""Exercise the queue against isolated FTPS/MQTT peers and restart the real server.

python3 tests/queue_printer.py BINARY OUTPUT_DIR
"""
import concurrent.futures
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
import urllib.request
import uuid
from printer_mqtt import SERIAL, SECRET, until
from printer_start import ARTIFACT, PrintBroker, Ftps, REPO


def main():
    binary = str(Path(sys.argv[1]).resolve())
    output = Path(sys.argv[2]).resolve()
    output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='orca-queue-test-') as directory:
        tmp=Path(directory)
        subprocess.run(['openssl','req','-x509','-newkey','ec','-pkeyopt','ec_paramgen_curve:P-256','-nodes','-days','1',
                        '-subj','/CN=isolated-printer','-keyout',str(tmp/'key'),'-out',str(tmp/'cert')],
                       check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        broker=PrintBroker(tmp/'cert',tmp/'key')
        ftp=Ftps(tmp/'cert',tmp/'key')
        plate_id, revision=str(uuid.uuid4()), str(uuid.uuid4())
        store=tmp/'plates'; data=store/plate_id/'revisions'/revision; data.mkdir(parents=True)
        for index in range(2): (data/f'{index}.stl').write_bytes((REPO/'tests/fixtures/cube.stl').read_bytes())
        for name in ['project.3mf','print.gcode.3mf']: (data/name).write_bytes(ARTIFACT)
        plate=dict(format_version=1,id=plate_id,revision=revision,name='Original plate',settings={'material':'before'},
                   models=[dict(name=f'cube-{i}.stl',path=f'revisions/{revision}/{i}.stl',source=None) for i in range(2)],
                   project=f'revisions/{revision}/project.3mf',print=f'revisions/{revision}/print.gcode.3mf')
        metadata=store/plate_id/'plate.json'; metadata.write_text(json.dumps(plate))
        full=json.loads((REPO/'tests/fixtures/p1_status.json').read_text())
        full['print']['ams']['tray_exist_bits']='9'
        full['print']['ams']['ams'][0]['tray'][3].update(tray_type='PLA',tray_color='00FFFFFF')
        with socket.socket() as sock: sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
        env={k:v for k,v in os.environ.items() if not k.startswith('P1_') and k not in ('ORCA_APPDIR','SCAD_LIVE_URL')}
        env.update(PORT=str(port),PLATES_DIR=str(store),LOG_LEVEL='trace',P1_IP='127.0.0.1',P1_SERIAL=SERIAL,
                   P1_ACCESS_CODE=SECRET,P1_TLS_CERT=str(tmp/'cert'),P1_MQTT_PORT=str(broker.port),
                   P1_FTPS_PORT=str(ftp.port),P1_START_TIMEOUT_SECS='10')
        def launch():
            log=(output/'server.log').open('a')
            return subprocess.Popen([binary],cwd=tmp,env=env,stdout=log,stderr=log),log
        def stop(process,log): process.terminate(); process.wait(timeout=5); log.close()
        def api(path='/api/queue',body=None,origin=None,expected=200):
            headers={'Content-Type':'application/json'}
            if origin: headers['Origin']=origin
            req=urllib.request.Request(f'http://127.0.0.1:{port}'+path,data=json.dumps(body).encode() if body is not None else None,headers=headers)
            try: response=urllib.request.urlopen(req,timeout=5)
            except urllib.error.HTTPError as error: response=error
            with response:
                raw=response.read(); assert SECRET.encode() not in raw
                if expected is not None: assert response.code == expected,(response.code,raw)
                return response.code,json.loads(raw)
        def state(): return api()[1]
        def command(action,base=None):
            return dict(generation=(base if base is not None else state())['generation'],request_id=str(uuid.uuid4()),action=action)
        def send(action,expected=200): return api(body=command(action),expected=expected)[1]
        def add(slot=3): return send(dict(type='add',plate_id=plate_id,revision=revision,ams_slot=slot))
        def report(command,state_name):
            value=copy.deepcopy(full)
            value['print'].update(gcode_state=state_name,subtask_name=command['subtask_name'],gcode_file=command['file'])
            broker.send(value)
        def phase(name): return until(lambda: state()['current'] and state()['current']['phase']==name)
        process,log=launch()
        try:
            until(lambda: len(broker.requests)==1)
            first=add(); a=first['waiting'][0]['id']
            # Queueing does not require an online printer; sending does.
            send(dict(type='next',expected_job=a,cleared=True),409)
            broker.send(full); until(lambda: state()['printer']['ready_to_print'])
            request=command(dict(type='add',plate_id=plate_id,revision=revision,ams_slot=0))
            api(body=request); repeated=api(body=request)[1]
            assert len(repeated['waiting'])==2
            b=repeated['waiting'][1]['id']
            c=add()['waiting'][-1]['id']
            send(dict(type='move',job_id=c,index=0)); assert state()['waiting'][0]['id']==c
            send(dict(type='remove',job_id=c)); assert [j['id'] for j in state()['waiting']]==[a,b]
            api(body=command(dict(type='next',expected_job=a,cleared=True)),origin='https://foreign.invalid',expected=403)
            send(dict(type='next',expected_job=a,cleared=False),409)
            # Destroy the editable source artifacts after enqueue. Both jobs must still upload their frozen bytes.
            for name in ['project.3mf','print.gcode.3mf']: (data/name).write_bytes(b'changed source')
            edited=copy.deepcopy(plate); edited.update(name='Edited plate',settings={'material':'after'},print=None,project=None)
            metadata.write_text(json.dumps(edited))
            request_a=command(dict(type='next',expected_job=a,cleared=True))
            api(body=request_a)
            until(lambda: len(broker.prints)==1)
            cmd_a=broker.prints[0]; assert cmd_a['ams_mapping']==[3]
            report(cmd_a,'RUNNING'); phase('printing')
            assert state()['current']['job']['name']=='Original plate'
            send(dict(type='next',expected_job=b,cleared=True),409)
            report(cmd_a,'FINISH'); phase('awaiting_removal')
            time.sleep(.3); assert len(broker.prints)==1
            api(body=request_a); assert len(broker.prints)==1
            base=state()
            next1=command(dict(type='next',expected_job=b,cleared=True),base)
            next2=command(dict(type='next',expected_job=b,cleared=True),base)
            with concurrent.futures.ThreadPoolExecutor(2) as pool:
                replies=list(pool.map(lambda req:api(body=req,expected=None),[next1,next2]))
            assert sorted(code for code,_ in replies)==[200,409]
            accepted=next1 if replies[0][0]==200 else next2
            until(lambda: len(broker.prints)==2)
            cmd_b=broker.prints[1]; assert cmd_b['ams_mapping']==[0]
            api(body=accepted); time.sleep(.2); assert len(broker.prints)==2
            api(body=request_a,expected=409)
            report(cmd_b,'RUNNING'); phase('printing')
            broker.actions.put('disconnect'); phase('needs_attention')
            send(dict(type='next',expected_job=b,cleared=True),409)
            until(lambda: len(broker.requests)==2)
            assert len(broker.prints)==2
            send(dict(type='retry',expected_job=b,cleared=True),409)
            report(cmd_b,'RUNNING'); phase('printing')
            report(cmd_b,'IDLE'); phase('needs_attention')
            send(dict(type='next',expected_job=b,cleared=True),409)
            # A stopped print requires a checked retry, retains its frozen snapshot, and sends once.
            send(dict(type='retry',expected_job=b,cleared=False),409)
            retry=command(dict(type='retry',expected_job=b,cleared=True))
            api(body=retry); until(lambda: len(broker.prints)==3)
            api(body=retry); assert len(broker.prints)==3
            cmd_retry=broker.prints[-1]
            report(cmd_retry,'RUNNING'); phase('printing')
            # Restart loses only the queue, keeps source plates, and cannot start while P1 is busy.
            stop(process,log); process,log=launch()
            until(lambda: len(broker.requests)==3)
            empty=state(); assert empty['current'] is None and empty['waiting']==[] and not empty['printer']['ready_to_print']
            assert api('/api/plates/'+plate_id)[1]['name']=='Edited plate'
            for name in ['project.3mf','print.gcode.3mf']: (data/name).write_bytes(ARTIFACT)
            metadata.write_text(json.dumps(plate))
            new=add(); new_id=new['waiting'][0]['id']
            api(body=accepted,expected=409)
            send(dict(type='next',expected_job=new_id,cleared=True),409)
            report(cmd_retry,'RUNNING'); until(lambda: state()['printer']['print']['state']=='RUNNING')
            send(dict(type='next',expected_job=new_id,cleared=True),409)
            assert len(broker.prints)==3
            broker.send(full); until(lambda: state()['printer']['ready_to_print'])
            ftp.actions.put('fail')
            send(dict(type='next',expected_job=new_id,cleared=True)); phase('needs_attention')
            assert len(broker.prints)==3
            send(dict(type='discard',expected_job=new_id,cleared=False),409)
            send(dict(type='discard',expected_job=new_id,cleared=True)); assert state()['current'] is None
        finally:
            stop(process,log); ftp.close(); broker.close()
        for path in output.glob('*.log'): assert SECRET.encode() not in path.read_bytes()
    result=dict(frozen_source_and_ams=True,reorder_remove=True,add_replay_once=True,
                completion_waits_for_removal=True,disconnect_needs_attention_no_replay=True,concurrent_next_once=True,old_and_replayed_next_no_extra_print=True,
                stopped_print_requires_recovery=True,explicit_retry_once=True,upload_failure_needs_attention=True,
                restart_empty_source_preserved=True,restart_busy_or_unknown_no_start=True,origin_checked=True,secrets_redacted=True)
    (output/'result.json').write_text(json.dumps(result,indent=2)); print(json.dumps(result))


if __name__=='__main__': main()
