"""Automatic estimate lifecycle with isolated source, CLI, MQTT, FTPS and SQLite.

python3 tests/queue_estimates.py BINARY OUTPUT_DIR
"""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import shutil
import subprocess
import sys
import time
from unittest.mock import patch
from print_fixture import Rig, REPO, printer_control
from printer_mqtt import until


def run(binary, output):
    with patch.dict(os.environ,DISCORD_WEBHOOK_URL="invalid-fixture-ambient",ORCA_PUBLIC_URL="https://fixture.invalid"):
        rig=Rig(binary,output)
    results={};control=None
    def waiting(job): return next(j for j in rig.api()['waiting'] if j['id']==job['id'])
    def ready(job):
        until(lambda:waiting(job)['estimate']['state']=='ready',30)
        assert waiting(job)['estimate']['seconds']==1140
    def saved(job):
        with sqlite3.connect(rig.store/'orca.sqlite3') as c:return json.loads(c.execute('SELECT estimate_json FROM print_jobs WHERE id=?',(job['id'],)).fetchone()[0])
    def cache(job):return rig.store/'jobs'/job['id']/('estimate-'+saved(job)['id'])
    def hold(on):
        path=rig.root/'cli-hold';path.touch() if on else path.unlink(missing_ok=True)
    def traces():return rig.traces() if (rig.root/'cli.jsonl').exists() else []
    def remove_current():rig.send(dict(type='discard',expected_job=rig.api()['current']['id'],cleared=True))
    def finish():rig.report('RUNNING');rig.phase('printing');rig.report('FINISH');rig.phase('awaiting_removal')
    def edit():
        plate=rig.api('/api/plates/'+rig.plate['id']);plate['name']+=' updated'
        rig.plate=rig.api('/api/plates/'+plate.pop('id'),plate,'PUT')
    try:
        assert "DISCORD_WEBHOOK_URL" not in rig.env and "ORCA_PUBLIC_URL" not in rig.env, "isolate ambient notification settings"
        rig.launch();rig.seed();hold(True)
        command=rig.command(rig.add_action());started=time.monotonic();q=rig.api(body=command)
        assert time.monotonic()-started<1 and q['waiting'][0].get('estimate'), 'Add must return estimate state promptly'
        first=q['waiting'][0];rig.api(body=command);assert len(rig.api()['waiting'])==1
        until(lambda:waiting(first)['estimate']['state']=='calculating')
        assert not rig.broker.prints and not rig.ftp.uploads
        hold(False);ready(first);assert len(traces())==2
        persisted=saved(first);rig.stop();rig.launch();rig.idle();ready(first)
        time.sleep(.4);assert saved(first)==persisted and len(traces())==2
        before=len(traces());rig.next(first);until(lambda:len(rig.broker.prints)==1)
        assert len(traces())==before, (before,len(traces()))
        assert rig.api()['current']['estimate']['seconds']==1140, rig.api()['current']['estimate']
        rig.report('RUNNING');rig.phase('printing')
        results['automatic_nonblocking_add_replay_restart_and_cache_reuse']=True
        hold(True);second=rig.add();until(lambda:waiting(second)['estimate']['state']=='calculating')
        assert rig.api()['current']['id']==first['id'] and len(rig.broker.prints)==len(rig.ftp.uploads)==1
        old_cache=cache(second);edit();assert waiting(second)['estimate']==dict(state='pending',seconds=None,error=None)
        hold(False);ready(second);assert not old_cache.exists()
        assert saved(second)['input']['plate']['name']==rig.plate['name']
        finish();remove_current();results['estimate_while_printing_and_discard_stale_completion']=True
        # A known material setting change hides the old duration immediately and re-slices.
        setting=rig.api('/api/filaments/'+rig.materials[1]['id'])['settings'][0]
        data={k:setting[k] for k in ['machine_profile_key','base_profile_key','overrides_json']};data['overrides_json']['nozzle_temperature']=220
        hold(True);rig.api('/api/filaments/'+rig.materials[1]['id']+'/settings/'+setting['id'],data,'PUT')
        assert waiting(second)['estimate']['seconds'] is None
        hold(False);ready(second);assert saved(second)['input']['settings']['profiles']['filament.json']['nozzle_temperature']==['220']
        # SCAD has no revision feed: fetch latest again at Next and compare actual bytes.
        rig.files['parts/cube.stl']=b'Updated'+rig.files['parts/cube.stl'][7:]
        before=len(traces());rig.next(second);until(lambda:len(rig.broker.prints)==2)
        assert len(traces())==before+2, (before,len(traces()),traces())
        expected=hashlib.sha256(rig.files['parts/cube.stl']).hexdigest();assert traces()[-1]['inputs']==[expected,expected]
        finish();remove_current();results['known_profile_changes_and_latest_scad_start_reslice']=True
        # A killed calculation restarts; successful unrelated entries are not replayed.
        hold(True);third=rig.add();until(lambda:waiting(third)['estimate']['state']=='calculating')
        old=cache(third);rig.stop(kill=True);rig.launch();rig.idle()
        until(lambda: saved(third)['id']!=old.name.removeprefix('estimate-'))
        hold(False);ready(third);assert not old.exists() and len(rig.broker.prints)==2
        # A damaged cached artifact cannot be sent to the printer.
        (cache(third)/'print.gcode.3mf').write_bytes(b'broken')
        before=len(traces());rig.next(third);until(lambda:len(rig.broker.prints)==3)
        assert len(traces())==before+2;finish();remove_current()
        results['interrupted_calculation_recovers_and_bad_cache_reslices']=True
        hold(True);cancelled=rig.add();until(lambda:waiting(cancelled)['estimate']['state']=='calculating')
        rig.send(dict(type='remove',job_id=cancelled['id']));assert not (rig.store/'jobs'/cancelled['id']).exists()
        hold(False);failure=rig.root/'cli-fail';failure.touch();failed=rig.add()
        until(lambda:waiting(failed)['estimate']['state']=='failed')
        assert waiting(failed)['estimate']['seconds'] is None
        failure.unlink();rig.send(dict(type='reestimate',job_id=failed['id']));ready(failed)
        assert len(rig.broker.prints)==len(rig.ftp.uploads)==3
        assert not (rig.store/'jobs'/cancelled['id']).exists()
        results['cancel_cleanup_and_explicit_failure_recovery_without_print']=True
        # An unwritable job path is a per-job failure, not a stuck worker for the whole queue.
        blocked=rig.store/'jobs'/failed['id'];shutil.rmtree(blocked);blocked.write_bytes(b'blocked-directory')
        rig.send(dict(type='reestimate',job_id=failed['id']))
        until(lambda:waiting(failed)['estimate']['state']=='failed',3)
        assert waiting(failed)['estimate']['seconds'] is None
        following=rig.add();ready(following)
        rig.send(dict(type='remove',job_id=following['id']))
        blocked.unlink();rig.send(dict(type='reestimate',job_id=failed['id']));ready(failed)
        results['storage_failure_is_visible_and_does_not_starve_other_jobs']=True
        # Upgrade a prior-schema queued job; calculate without starting a print.
        rig.stop()
        with sqlite3.connect(rig.store/'orca.sqlite3') as c:
            c.execute('ALTER TABLE print_jobs DROP COLUMN estimate_json');c.execute('PRAGMA user_version=8')
        rig.launch();rig.idle();ready(failed);assert len(rig.broker.prints)==3
        rig.send(dict(type='remove',job_id=failed['id']));results['schema_eight_queued_job_recovers_without_print']=True
        if os.environ.get('ESTIMATE_BROWSER'):
            control=printer_control(rig)
            env=dict(os.environ,E2E_BASE_URL=rig.base,E2E_ESTIMATE_CONTEXT=json.dumps(dict(plate_id=rig.plate['id'])),E2E_PRINTER_CONTROL=f'http://127.0.0.1:{control.server_port}',E2E_EVIDENCE_DIR=str(rig.output))
            subprocess.run(['npm','run','test:e2e','--','estimates-live.spec.ts','--workers=1'],cwd=REPO/'client',env=env,check=True)
            results['chromium']=True
        assert not rig.ftp.errors
        results.update(print_commands=len(rig.broker.prints),uploads=len(rig.ftp.uploads),estimate_seconds=1140)
        (rig.output/'result.json').write_text(json.dumps(results,indent=2));print(json.dumps(results))
    finally:
        hold(False)
        if control:control.shutdown();control.server_close()
        rig.close()


if __name__=='__main__':run(sys.argv[1],sys.argv[2])
