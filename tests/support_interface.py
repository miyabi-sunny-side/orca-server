"""Support selection, frozen two-material execution and recovery against isolated peers."""
import copy
import json
import os
import sqlite3
import subprocess
import sys
from pathlib import Path
from print_fixture import Rig, MACHINE, REPO, until


def legacy(binary, output):
    rig = Rig(binary, output)
    rig.env['P1_START_TIMEOUT_SECS'] = '2'
    try:
        rig.launch(); rig.seed(); job = rig.add(0); rig.next(job)
        until(lambda:(rig.api()['printer']['start'] or {}).get('phase')=='unknown')
        assert len(rig.broker.prints)==1
        rig.stop()
        # Recreate the schema-12 one-material persistence format, including an uncertain start.
        with sqlite3.connect(rig.store/'orca.sqlite3') as db:
            execution = json.loads(db.execute('SELECT execution_json FROM print_jobs WHERE id=?',(job['id'],)).fetchone()[0])
            for key in ['interface','setting','ams_slot_id']:
                execution.pop(key,None)
            execution['profiles']['filament.json'].pop('filament_colour',None)
            for key in ['support_enabled','support_interface_filament_id']:
                execution['plate']['conditions'].pop(key)
                db.execute('ALTER TABLE plates DROP COLUMN '+key)
            db.execute('UPDATE print_jobs SET execution_json=?,estimate_json=NULL WHERE id=?',(json.dumps(execution),job['id']))
            db.execute('DROP TABLE plate_imports')
            db.execute('PRAGMA user_version=12')
        rig.launch(); rig.idle(); rig.phase('needs_attention')
        assert len(rig.broker.prints)==1
        assert rig.api()['current']['id']==job['id']
        plate = rig.api('/api/plates/'+rig.plate['id'])
        assert plate['conditions']['support_enabled'] is False
        assert plate['conditions']['support_interface_filament_id'] is None
        with sqlite3.connect(rig.store/'orca.sqlite3') as db:
            assert db.execute('PRAGMA user_version').fetchone()[0]==14
        rig.send(dict(type='retry',expected_job=job['id'],cleared=True))
        until(lambda:len(rig.broker.prints)==2)
        assert rig.broker.prints[-1]['ams_mapping']==[0]
        rig.report('RUNNING'); rig.phase('printing'); rig.stop(); rig.launch()
        rig.report('RUNNING'); rig.phase('printing')
        assert len(rig.broker.prints)==2
        (rig.output/'result.json').write_text(json.dumps(dict(schema12_to13=True,old_execution_attempt_restored=True,uncertain_no_replay=True,explicit_retry_mapping=[0],running_restart=True)))
    finally:rig.close()


def run(binary, output):
    rig = Rig(binary, output)
    rig.env['P1_START_TIMEOUT_SECS'] = '3'
    def stored(job, column):
        with sqlite3.connect(rig.store/'orca.sqlite3') as c:
            return c.execute(f'SELECT {column} FROM print_jobs WHERE id=?', (job['id'],)).fetchone()[0]
    def edit(changes):
        plate = rig.api('/api/plates/'+rig.plate['id']); plate['conditions'].update(changes)
        rig.plate = rig.api('/api/plates/'+plate.pop('id'), plate, 'PUT')
        return rig.plate
    def ready(job):
        def finished():
            estimate = next(j for j in rig.api()['waiting'] if j['id']==job['id'])['estimate']
            assert estimate['state'] != 'failed', estimate
            return estimate['state'] == 'ready'
        until(finished, 30)
        return json.loads(stored(job, 'estimate_json'))
    def map_blue():
        until(lambda:next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==3)['reported']['present'] is True)
        slot = next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==3)
        rig.api('/api/printers/p1/ams/'+slot['id'], dict(revision=slot['revision'],filament_id=rig.materials[1]['id']), 'PUT', 204)
    def temperature(index, value):
        fid = rig.materials[index]['id']; setting = rig.api('/api/filaments/'+fid)['settings'][0]
        data = {k:setting[k] for k in ['machine_profile_key','base_profile_key','overrides_json']}
        data['overrides_json']['nozzle_temperature'] = value
        rig.api('/api/filaments/'+fid+'/settings/'+setting['id'], data, 'PUT')
    def retry(job):
        return rig.send(dict(type='retry',expected_job=job['id'],cleared=True))
    def stopped():
        until(lambda:rig.api()['printer']['start']['phase']=='not_sent')
        assert not rig.broker.prints
    try:
        rig.launch(); rig.seed()
        white, blue = [f['id'] for f in rig.materials]
        gf = rig.api('/api/filaments',dict(name='接触面用の長い名前・PETG-GF 黒',vendor='Fixture',material='PETG-GF',color='000000FF',bambu_filament_id=None),expected=201)
        rig.api('/api/filaments/'+gf['id']+'/settings',dict(machine_profile_key=MACHINE,base_profile_key='Generic PETG',overrides_json={}),expected=201)
        assert rig.plate['conditions']['support_enabled'] is False
        assert rig.plate['conditions']['support_interface_filament_id'] is None
        edit(dict(support_enabled=True))
        assert rig.plate['conditions']['support_interface_filament_id']==white
        job=rig.send(dict(type='add',plate_id=rig.plate['id'],plate_version=rig.plate['version']))['waiting'][0]
        first=ready(job); assert 'interface' not in first['input']['settings']
        assert first['input']['settings']['profiles']['process.json']['support_interface_filament']=='1'
        edit(dict(support_interface_filament_id=blue)); second=ready(job)
        assert second['id']!=first['id']
        settings=second['input']['settings'];assert settings['ams_slot']==0 and settings['interface']['ams_slot']==3
        assert settings['profiles']['process.json']['support_interface_filament']=='2'
        assert settings['profiles']['filament.json']['filament_colour']==['#FFFFFF']
        assert settings['profiles']['interface.json']['filament_colour']==['#00FFFF']
        for mutate in [lambda:temperature(1,225),map_blue,lambda:temperature(0,216)]:
            mutate(); current=ready(job);assert current['id']!=second['id'];second=current
        edit(dict(support_enabled=False));assert edit(dict(support_enabled=True))['conditions']['support_interface_filament_id']==blue
        second=ready(job)
        # Missing interface blocks admission and the estimate without substituting the main material.
        slot=next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==3)
        rig.api('/api/printers/p1/ams/'+slot['id'],dict(revision=slot['revision'],filament_id=None),'PUT',204)
        q=rig.api('/api/queue?printer_id=p1&plate_id='+rig.plate['id'])
        assert not q['admission']['allowed'] and 'support interface' in q['admission']['reason']
        assert q['waiting'][0]['estimate']['state']=='failed'
        assert not rig.broker.prints and not rig.ftp.uploads
        map_blue(); ready(job)
        subprocess.run(['cargo','test','--locked','--test','mcp','support_plate_conditions','--','--ignored'],cwd=REPO,env={**os.environ,'MCP_FIXTURE_URL':rig.base,'MCP_SUPPORT_PLATE':rig.plate['id'],'MCP_SUPPORT_INTERFACE':blue},check=True)
        ready(job)
        if os.environ.get('SUPPORT_BROWSER'):
            subprocess.run(['npm','run','test:e2e','--','--workers=1'],cwd=REPO/'client',env={**os.environ,'E2E_BASE_URL':rig.base,'E2E_SUPPORT_CONTEXT':json.dumps(dict(white=white,blue=blue,gf=gf['id'])),'E2E_EVIDENCE_DIR':str(rig.output/'browser')},check=True)
        # Upload barrier must reject a revision change on the secondary slot.
        before=len(rig.traces());rig.ftp.actions.put('wait');rig.next(job)
        assert rig.ftp.received.wait(10)
        assert len(rig.traces())==before, 'Preparation must reuse the matching two-material estimate'
        frozen=stored(job,'execution_json')
        edit(dict(support_interface_filament_id=gf['id']))
        assert stored(job,'execution_json')==frozen
        map_blue();rig.ftp.gate.set();stopped()
        # Also reject a newly absent secondary, even after an explicit retry.
        rig.ftp.received.clear();rig.ftp.gate.clear();rig.ftp.actions.put('wait');retry(job)
        assert rig.ftp.received.wait(10)
        absent=copy.deepcopy(rig.full);absent['print']['ams']['tray_exist_bits']='1';rig.broker.send(absent)
        until(lambda:not next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index']==3)['reported']['present'])
        rig.ftp.gate.set();stopped();rig.idle();map_blue()
        # A common-setting change on either material is fenced before MQTT publication.
        for index, old in [(1,225),(0,216)]:
            rig.ftp.received.clear();rig.ftp.gate.clear();rig.ftp.actions.put('wait');retry(job)
            assert rig.ftp.received.wait(10)
            temperature(index,old+1);rig.ftp.gate.set();stopped();temperature(index,old)
        retry(job);until(lambda:len(rig.broker.prints)==1)
        assert rig.broker.prints[0]['ams_mapping']==[0,3]
        active=json.loads(stored(job,'execution_json'))
        assert active['interface']['filament']['id']==blue and active['plate']['conditions']['support_interface_filament_id']==blue
        assert active['profiles']==json.loads(frozen)['profiles']
        until(lambda:rig.api()['printer']['start']['phase']=='unknown')
        rig.stop();rig.launch();rig.idle()
        assert rig.api()['current']['state']=='needs_attention'
        assert len(rig.broker.prints)==1
        result=dict(same_id_deduplicated=True,distinct_pla_preserved=True,independent_cache_invalidation=True,missing_interface_blocks=True,mcp_rest=True,slot_revision_absence_and_both_setting_changes_no_send=True,frozen_plate_and_profiles=True,ordered_mapping=[0,3],unknown_restart_no_replay=True,real_printer_commands=0,isolated_print_commands=1)
        (rig.output/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
    finally:rig.close()


if __name__=='__main__':
    legacy(sys.argv[1],Path(sys.argv[2])/'legacy')
    run(*sys.argv[1:])
