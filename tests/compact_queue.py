"""Compact queue recovery and logical deletion with isolated HTTP/SQLite/MQTT/FTPS."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
from print_fixture import Rig, MACHINE, REPO, printer_control
from printer_mqtt import until


def run(binary, output):
    rig = Rig(binary, output)
    control = None
    results = {}
    def rows(sql, args=()):
        with sqlite3.connect(rig.store/'orca.sqlite3') as c:
            return c.execute(sql, args).fetchall()
    def ready():
        def finished():
            jobs = rig.api()['waiting']
            assert all(j['estimate']['state'] != 'failed' for j in jobs), jobs
            return all(j['estimate']['state'] == 'ready' for j in jobs)
        until(finished, 30)
    try:
        rig.full['print']['ams']['ams'][0]['tray'][0]['tray_type'] = 'PETG'
        rig.launch(); rig.seed()
        active = rig.add()
        active_plate = rig.plate
        material = rig.api('/api/filaments', dict(name='PETG-GF 黒', vendor='Fixture', material='PETG-GF', color='FFFFFFFF', bambu_filament_id=None), expected=201)
        data = dict(machine_profile_key=MACHINE, base_profile_key='Generic PETG', overrides_json=dict(nozzle_temperature_initial_layer=250, nozzle_temperature=240, bed_temperature_initial_layer=65, bed_temperature=65))
        setting = rig.api(f'/api/filaments/{material["id"]}/settings', data, expected=201)
        slot = next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index'] == 0)
        rig.api(f'/api/printers/p1/ams/{slot["id"]}', dict(revision=slot['revision'], filament_id=material['id']), 'PUT', 204)
        spec = dict(rig.specification(0, material['id']), bed_type='Cool Plate')
        conditions = {k:v for k,v in spec.items() if k != 'ams_slot_id'}
        plate = rig.api('/api/plates/import', dict(name='PETG-GF 前側ケース', models=[dict(name='parts/cube.stl', source='parts/cube.stl', quantity=2)], conditions=conditions), expected=201)
        action = dict(type='add', plate_id=plate['id'], plate_version=plate['version'])
        one = rig.send(action)['waiting'][-1]; two = rig.send(action)['waiting'][-1]
        ready()
        originals = rows('SELECT * FROM plate_items ORDER BY id')
        ids = [j['id'] for j in rig.api()['waiting']]
        rig.stop()
        with sqlite3.connect(rig.store/'orca.sqlite3') as c:
            c.executescript('DROP TABLE plate_imports; ALTER TABLE plates DROP COLUMN support_interface_filament_id; ALTER TABLE plates DROP COLUMN support_enabled; ALTER TABLE plates DROP COLUMN brim_enabled; ALTER TABLE plates DROP COLUMN deleted; PRAGMA user_version=10;')
        rig.launch(); rig.idle(); ready()
        assert rows('PRAGMA user_version') == [(14,)]
        assert rows('SELECT * FROM plate_items ORDER BY id') == originals
        assert [j['id'] for j in rig.api()['waiting']] == ids
        results['schema_10_to_12_preserves_compositions_and_queue'] = True
        rig.next(active); until(lambda: len(rig.broker.prints) == 1)
        rig.report('RUNNING'); rig.phase('printing')
        frozen = rows('SELECT execution_json,attempt_json,artifact_path FROM print_jobs WHERE id=?', (active['id'],))
        before = (len(rig.broker.prints), len(rig.ftp.uploads))
        data['overrides_json'].update(bed_temperature_initial_layer=0, bed_temperature=0)
        rig.api(f'/api/filaments/{material["id"]}/settings/{setting["id"]}', data, 'PUT')
        reason = 'Selected build plate temperature is missing or zero for this material'
        until(lambda: all(j['estimate']['state']=='failed' and j['estimate']['error']==reason for j in rig.api()['waiting']))
        admission = rig.api('/api/queue?printer_id=p1&plate_id='+plate['id'])['admission']
        assert not admission['allowed'] and admission['reason'] == reason
        if os.environ.get('COMPACT_BROWSER'):
            control = printer_control(rig)
            env = dict(os.environ, E2E_BASE_URL=rig.base, E2E_COMPACT_CONTEXT=json.dumps(dict(plate=plate['id'], material=material['id'], job=one['id'])), E2E_EVIDENCE_DIR=str(rig.output), E2E_PRINTER_CONTROL=f'http://127.0.0.1:{control.server_port}')
            subprocess.run(['npm','run','test:e2e','--','--workers=1'], cwd=REPO/'client', env=env, check=True)
        else:
            data['overrides_json'].update(bed_temperature_initial_layer=65, bed_temperature=65)
            rig.api(f'/api/filaments/{material["id"]}/settings/{setting["id"]}', data, 'PUT')
            rig.api('/api/plates/'+plate['id'], method='DELETE', expected=204)
        ready()
        saved = rig.api('/api/filaments/'+material['id'])['settings'][0]['overrides_json']
        assert saved == dict(nozzle_temperature_initial_layer=250, nozzle_temperature=240, bed_temperature_initial_layer=65, bed_temperature=65)
        results['cool_plate_zero_explained_and_65_recovery_preserves_nozzle'] = True
        assert (len(rig.broker.prints), len(rig.ftp.uploads)) == before
        rig.api('/api/plates/'+active_plate['id'], method='DELETE', expected=204)
        for p in [plate, active_plate]:
            rig.api('/api/plates/'+p['id'], expected=404)
            rig.api('/api/plates/'+p['id'], dict(name=p['name'],version=p['version'],models=p['models'],conditions=p['conditions']), 'PUT', 409)
            assert not any(x['id']==p['id'] for x in rig.api('/api/plates?q='+p['id']))
        rig.send(action, expected=409)
        assert rows('SELECT * FROM plate_items ORDER BY id') == originals
        assert rows('SELECT execution_json,attempt_json,artifact_path FROM print_jobs WHERE id=?', (active['id'],)) == frozen
        q = rig.api()
        assert q['current']['plate_deleted'] and all(j['plate_deleted'] for j in q['waiting'])
        stale = rig.command(dict(type='move',job_id=one['id'],index=1), q)
        rig.send(dict(type='move',job_id=two['id'],index=0))
        rig.api(body=stale, expected=409)
        assert rig.api()['waiting'][0]['id']==two['id']
        assert (len(rig.broker.prints), len(rig.ftp.uploads)) == before
        results['delete_and_reorder_preserve_frozen_input_and_send_nothing'] = True
        rig.report('FINISH'); rig.phase('awaiting_removal')
        rig.next(two); until(lambda:len(rig.broker.prints)==2)
        assert rig.broker.prints[-1]['ams_mapping']==[0]
        rig.report('RUNNING');rig.phase('printing')
        rig.stop();rig.launch();rig.idle()
        assert rig.api('/api/plates') == []
        assert len(rig.api()['waiting']) == 1 and rig.api()['current']['plate_deleted']
        assert len(rig.broker.prints)==len(rig.ftp.uploads)==2
        results['deleted_existing_job_starts_only_on_explicit_next_and_persists'] = True
        (rig.output/'result.json').write_text(json.dumps(results, indent=2))
        print(json.dumps(results))
    finally:
        if control: control.shutdown();control.server_close()
        rig.close()


if __name__ == '__main__': run(sys.argv[1], sys.argv[2])
