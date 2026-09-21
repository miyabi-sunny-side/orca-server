"""Durable queue against isolated HTTP, SCAD, MQTT and FTPS peers.

python3 tests/queue_printer.py BINARY OUTPUT_DIR
"""
import concurrent.futures
import copy
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import sys
import time
import uuid
from print_fixture import Rig, FILAMENT
from printer_mqtt import until


def main():
    rig=Rig(sys.argv[1],sys.argv[2]); results={}
    try:
        rig.launch();rig.seed()
        first=rig.add();second=rig.add(0)
        request=rig.command(rig.add_action())
        rig.api(body=request);rig.api(body=request)
        assert len(rig.api()['waiting'])==3
        last=rig.api()['waiting'][-1]
        rig.send(dict(type='move',job_id=last['id'],index=0));assert rig.api()['waiting'][0]['id']==last['id']
        rig.send(dict(type='remove',job_id=last['id']))
        # Waiting jobs use current plate conditions. Incompatible or unloaded conditions hold Next.
        stale=rig.command(dict(type='next',expected_job=first['id'],removed_job=None,cleared=True))
        altered=rig.api('/api/plates/'+rig.plate['id']);altered.pop('id');altered['conditions']['filament_id']=None
        rig.plate=rig.api('/api/plates/'+rig.plate['id'],altered,'PUT')
        rig.api(body=stale,expected=409);rig.next(first,409)
        assert rig.api()['waiting'][0]['state']=='queued' and rig.api()['waiting'][0]['hold_reason']
        rig.configure()
        settings={k:v for k,v in rig.api('/api/printers/p1').items() if k not in ('id','status','machine','configuration_error')}
        requests=len(rig.broker.requests)
        changed=dict(settings,machine_profile_key='Bambu Lab P1S 0.2 nozzle')
        rig.api('/api/printers/p1',changed,'PUT');until(lambda:len(rig.broker.requests)>requests);rig.idle()
        rig.next(first,409);assert 'nozzle' in rig.api()['waiting'][0]['hold_reason']
        requests=len(rig.broker.requests)
        rig.api('/api/printers/p1',settings,'PUT');until(lambda:len(rig.broker.requests)>requests);rig.idle()
        # Connection changes invalidate manual AMS assignments; explicitly restore both slots.
        for slot in rig.api('/api/printers/p1/ams')['slots']:
            if slot['slot_index'] in (0,3):
                material=rig.materials[0 if slot['slot_index']==0 else 1]
                rig.api(f'/api/printers/p1/ams/{slot["id"]}',dict(revision=slot['revision'],filament_id=material['id']),'PUT',204)
        results['waiting_material_and_nozzle_mismatch_stays_editable']=True
        # Fresh bytes are selected at preparation, not enqueue; a failed source has no fallback.
        original=rig.files['parts/cube.stl'];rig.files.clear()
        rig.next(first);rig.phase('needs_attention');assert not rig.broker.prints
        rig.files['parts/cube.stl']=original.replace(b'facet normal',b'facet  normal')
        changed=hashlib.sha256(rig.files['parts/cube.stl']).hexdigest()
        rig.ftp.actions.put('wait')
        retry=rig.command(dict(type='retry',expected_job=first['id'],cleared=True))
        rig.api(body=retry);assert rig.ftp.received.wait(10)
        with sqlite3.connect(rig.store/'orca.sqlite3') as db:
            row=db.execute('SELECT attempt_id,attempt_json,execution_json FROM print_jobs WHERE id=?',(first['id'],)).fetchone()
            assert row[0] and json.loads(row[1])['phase']=='uploading'
            assert json.loads(row[2])['profiles']['filament.json']['nozzle_temperature']==['215']
        assert rig.traces()[-1]['inputs']==[changed,changed]
        current=rig.api()['current'];path=rig.store/current['artifact_path']/'print.gcode.3mf'
        assert rig.ftp.contents[-1]==path.read_bytes()
        # Crash before the FTPS completion reply: the reserved attempt must never be resumed.
        rig.stop(kill=True);rig.ftp.gate.set();rig.launch()
        assert not rig.api()['printer']['synchronized'];rig.idle();rig.phase('needs_attention')
        time.sleep(.2);assert not rig.broker.prints
        rig.api(body=retry,expected=409)
        results['latest_scad_and_overrides_frozen_before_upload']=True
        results['crash_before_send_never_resumes_upload_or_start']=True
        # Explicit recovery starts once; an HTTP replay cannot reserve another attempt.
        retry=rig.command(dict(type='retry',expected_job=first['id'],cleared=True));rig.api(body=retry)
        until(lambda:len(rig.broker.prints)==1);rig.api(body=retry);assert len(rig.broker.prints)==1
        before_ack=rig.output/'before-ack.sqlite3'
        with sqlite3.connect(rig.store/'orca.sqlite3') as source, sqlite3.connect(before_ack) as target:source.backup(target)
        rig.stop(kill=True);rig.launch();rig.idle();rig.phase('needs_attention')
        time.sleep(.2);assert len(rig.broker.prints)==1
        # Even a ready printer does not establish that an unknown start never ran.
        assert rig.api()['current']['state']=='needs_attention'
        rig.report('RUNNING');rig.phase('printing');rig.report('FINISH');rig.phase('awaiting_removal')
        # Restore an older DB snapshot after a completed physical print. No automatic replay.
        rig.stop();shutil.copyfile(before_ack,rig.store/'orca.sqlite3');rig.launch();rig.idle();rig.phase('needs_attention')
        assert len(rig.broker.prints)==1
        rig.api(body=retry,expected=409)
        rig.send(dict(type='discard',expected_job=first['id'],cleared=True))
        assert rig.api()['current'] is None and len(rig.api()['waiting'])==1
        assert not (rig.store/'jobs'/first['id']).exists()
        results['restart_and_backup_restore_do_not_replay_uncertain_starts']=True
        # Same-version concurrent Next: exactly one request reserves the next print.
        q=rig.api();action=dict(type='next',expected_job=second['id'],removed_job=None,cleared=True)
        one=rig.command(action,q);two=copy.deepcopy(one);two['request_id']=str(uuid.uuid4())
        rig.api(body=one,expected=403,origin='https://elsewhere.invalid')
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            replies=list(pool.map(lambda request:rig.api(body=request,expected=None),[one,two]))
        assert sorted(code for code,_ in replies)==[200,409]
        accepted=one if replies[0][0]==200 else two
        until(lambda:len(rig.broker.prints)==2);rig.api(body=accepted);assert len(rig.broker.prints)==2
        assert rig.broker.prints[-1]['ams_mapping']==[0]
        command=rig.broker.prints[-1]
        rig.broker.send({'print':dict(command='project_file',sequence_id=command['sequence_id'],result='success')})
        until(lambda:rig.api()['printer']['start']['phase']=='accepted')
        rig.stop(kill=True);rig.launch();rig.idle();rig.phase('needs_attention')
        assert len(rig.broker.prints)==2
        rig.report('RUNNING');rig.phase('printing');rig.stop();rig.launch()
        rig.report('RUNNING');rig.phase('printing');rig.report('FINISH');rig.phase('awaiting_removal')
        rig.stop();rig.launch();rig.idle();rig.phase('awaiting_removal')
        assert len(rig.broker.prints)==2
        old_removal=rig.command(dict(type='discard',expected_job=second['id'],cleared=True))
        rig.api(body=old_removal);rig.api(body=old_removal)
        assert rig.api()['current'] is None
        with sqlite3.connect(rig.store/'orca.sqlite3') as db:assert db.execute('SELECT count(*) FROM print_jobs').fetchone()[0]==0
        results.update(concurrent_next_once=True,old_requests_rejected=True,accepted_running_and_removal_survive_restart=True,terminal_rows_and_artifacts_cleaned=True)
        # Swap a physical tray while FTPS is paused. Its old material mapping must not authorize MQTT.
        job=rig.add();rig.ftp.received.clear();rig.ftp.gate.clear();rig.ftp.actions.put('wait')
        rig.next(job);assert rig.ftp.received.wait(10)
        swapped=copy.deepcopy(rig.full);swapped['print']['ams']['ams'][0]['tray'][3]['tray_color']='000000FF'
        rig.broker.send(swapped)
        until(lambda:next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==3)['filament_id'] is None)
        rig.ftp.gate.set();rig.phase('needs_attention');assert len(rig.broker.prints)==2
        results['ams_swap_during_transfer_prevents_start']=True
        # A crash after a terminal transaction but before file cleanup is recovered at startup.
        rig.stop()
        with sqlite3.connect(rig.store/'orca.sqlite3') as db:
            db.execute("UPDATE print_jobs SET state='cancelled' WHERE id=?",(job['id'],))
        assert (rig.store/'jobs'/job['id']).exists()
        rig.launch()
        assert not (rig.store/'jobs'/job['id']).exists()
        with sqlite3.connect(rig.store/'orca.sqlite3') as db:
            assert db.execute('SELECT count(*) FROM print_jobs').fetchone()[0]==0
        results['terminal_cleanup_recovered_after_crash']=True
        assert not rig.ftp.errors
    finally:rig.close()
    (Path(sys.argv[2])/'result.json').write_text(json.dumps(results,indent=2));print(json.dumps(results))


if __name__=='__main__':main()
