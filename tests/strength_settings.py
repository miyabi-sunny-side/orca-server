"""Strength defaults, migration, shared slicing and frozen execution with disposable peers."""
import json
import os
import sqlite3
import subprocess
import sys
from urllib.parse import urlencode
from print_fixture import Rig, MACHINE, PROCESS, REPO, until

KEYS = ['sparse_infill_pattern', 'sparse_infill_density', 'wall_loops']
DEFAULT = dict(zip(KEYS, ['adaptivecubic', 15, 2]))


def run(binary, output):
    rig = Rig(binary, output)
    def values(plate): return {k:plate['conditions'][k] for k in KEYS}
    def edit(changes):
        plate=rig.api('/api/plates/'+rig.plate['id']); plate['conditions'].update(changes)
        rig.plate=rig.api('/api/plates/'+plate.pop('id'),plate,'PUT'); return rig.plate
    def stored(job, column):
        with sqlite3.connect(rig.store/'orca.sqlite3') as c:
            return c.execute(f'SELECT {column} FROM print_jobs WHERE id=?',(job['id'],)).fetchone()[0]
    def ready(job):
        def finished():
            estimate=next(j for j in rig.api()['waiting'] if j['id']==job['id'])['estimate']
            assert estimate['state']!='failed',estimate
            return estimate['state']=='ready'
        until(finished,30)
        return json.loads(stored(job,'estimate_json'))
    try:
        path=rig.root/'app/resources/profiles/BBL/process/standard.json'
        alternate=json.loads(path.read_text());alternate.update(name='0.16mm Fixture quality',top_shell_layers='7',bottom_shell_layers='4',top_shell_thickness='0.9')
        path.with_name('quality.json').write_text(json.dumps(alternate))
        rig.launch(); rig.seed()
        defaults=rig.api('/api/default-settings'); assert values(defaults)==DEFAULT
        assert values(rig.plate)==DEFAULT
        patterns=defaults['infill_patterns']; assert len(patterns)==len(set(patterns))==26
        for pattern in patterns:
            preview=rig.api('/api/slicer/process?'+urlencode(dict(machine=MACHINE,process=PROCESS,sparse_infill_pattern=pattern,sparse_infill_density=100,wall_loops=3)))
            assert preview['sparse_infill_pattern']==pattern and preview['sparse_infill_density']=='100%'
            assert preview['top_shell_layers']=='8' and preview['bottom_shell_layers']=='5'
        for change,status in [(dict(sparse_infill_pattern='unknown'),400),(dict(sparse_infill_density=100.1),400),(dict(sparse_infill_density=-1),400),(dict(wall_loops=2.5),422),(dict(wall_loops=-1),422),(dict(wall_loops=1001),400),(dict(unknown=True),422)]:
            rig.api('/api/default-settings',dict(default_printer_id='p1',**change),'PUT',status)
            plate=rig.api('/api/plates/'+rig.plate['id']);plate['conditions'].update(change)
            rig.api('/api/plates/'+plate.pop('id'),plate,'PUT',status)
        original=rig.plate.copy(); changed=dict(zip(KEYS,['gyroid',22.5,4]))
        rig.api('/api/default-settings',dict(default_printer_id='p1',**changed),'PUT',204)
        rig.api('/api/default-settings',dict(default_printer_id='p1'),'PUT',204)
        assert values(rig.api('/api/default-settings'))==changed
        assert rig.api('/api/plates/'+original['id'])==original
        new=rig.api('/api/plates/import',dict(name='New defaults',models=[dict(name='parts/cube.stl',source='parts/cube.stl',quantity=1)]),expected=201)
        assert values(new)==changed
        rig.stop();rig.launch();rig.idle()
        assert values(rig.api('/api/default-settings'))==changed and values(rig.api('/api/plates/'+new['id']))==changed
        # Existing MCP checks also exercise explicit overrides of these three fields.
        subprocess.run(['cargo','test','--locked','--test','mcp','creation_defaults_match_rest','--','--ignored'],cwd=REPO,env={**os.environ,'MCP_FIXTURE_URL':rig.base},check=True)
        job=rig.add(); first=ready(job)
        assert first['input']['settings']['profiles']['process.json']['sparse_infill_pattern']=='adaptivecubic'
        old=rig.store/'jobs'/job['id']/('estimate-'+first['id'])
        edit(changed); second=ready(job)
        assert first['id']!=second['id'] and not old.exists()
        process=second['input']['settings']['profiles']['process.json']
        assert (process['wall_loops'],process['top_shell_layers'],process['bottom_shell_layers'],process['top_shell_thickness'])==('4','10','6','2')
        assert process['sparse_infill_density']=='22.5%' and process['sparse_infill_pattern']=='gyroid'
        assert not rig.broker.prints and not rig.ftp.uploads
        traces=len(rig.traces());rig.next(job);until(lambda:len(rig.broker.prints)==1)
        assert len(rig.traces())==traces, 'Preparation must reuse the matching estimate'
        rig.report('RUNNING');rig.phase('printing')
        frozen=stored(job,'execution_json');attempt=stored(job,'attempt_json');artifact=stored(job,'artifact_path')
        edit(DEFAULT);rig.api('/api/default-settings',dict(default_printer_id='p1',**DEFAULT),'PUT',204)
        assert stored(job,'execution_json')==frozen and stored(job,'attempt_json')==attempt
        assert json.loads(frozen)['profiles']['process.json']==process
        # Manufacture schema 9 with an active attempt, a queued plate and nullable old conditions.
        queued=rig.add();ready(queued);rig.stop()
        with sqlite3.connect(rig.store/'orca.sqlite3') as c:
            for table in ['plates','default_settings']:
                for key in KEYS:c.execute(f'ALTER TABLE {table} DROP COLUMN {key}')
            c.execute('ALTER TABLE plates DROP COLUMN deleted')
            c.execute('PRAGMA user_version=9')
        rig.launch();rig.report('RUNNING');rig.phase('printing');ready(queued)
        assert stored(job,'execution_json')==frozen and stored(job,'attempt_json')==attempt and stored(job,'artifact_path')==artifact
        assert all(v is None for v in values(rig.api('/api/plates/'+rig.plate['id'])).values())
        assert values(rig.api('/api/default-settings'))==DEFAULT
        legacy=ready(queued)['input']['settings']['profiles']['process.json']
        assert (legacy['sparse_infill_pattern'],legacy['wall_loops'],legacy['top_shell_layers'])==('crosshatch','2','5')
        assert len(rig.broker.prints)==len(rig.ftp.uploads)==1
        rig.send(dict(type='remove',job_id=queued['id']))
        if os.environ.get('STRENGTH_BROWSER'):
            subprocess.run(['npm','run','test:e2e','--','--workers=1'],cwd=REPO/'client',env={**os.environ,'E2E_BASE_URL':rig.base,'E2E_STRENGTH_CONTEXT':json.dumps(dict(legacy=rig.plate['id'],material=rig.materials[0]['id'])),'E2E_EVIDENCE_DIR':str(rig.output/'browser')},check=True)
        result=dict(defaults_restart=True,rest_mcp=True,cli_patterns=26,validation=True,estimate_invalidates=True,preparation_reuses_exact_input=True,schema9_keeps_legacy=True,active_attempt_unchanged=True,isolated_print_commands=1)
        (rig.output/'result.json').write_text(json.dumps(result,indent=2)); print(json.dumps(result))
    finally:rig.close()


if __name__=='__main__':run(*sys.argv[1:])
