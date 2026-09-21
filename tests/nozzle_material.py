"""Material names and profile hardness must not infer installed-nozzle compatibility.

python3 tests/nozzle_material.py BINARY OUTPUT_DIR
Uses isolated HTTP/SQLite, SCAD, MQTT, FTPS and a recording slicer.
"""
import copy
import json
from pathlib import Path
import sys
from print_fixture import Rig, MACHINE
from printer_mqtt import until


def main():
    rig = Rig(sys.argv[1], sys.argv[2])
    try:
        profile_path = Path(rig.env['ORCA_APPDIR'])/'resources/profiles/BBL/filament/1.json'
        profile = json.loads(profile_path.read_text())
        profile.update(required_nozzle_HRC=['60'], compatible_printers=[MACHINE])
        profile_path.write_text(json.dumps(profile))
        rig.full['print'].update(nozzle_diameter='0.4', nozzle_type='stainless_steel')
        rig.full['print']['ams']['ams'][0]['tray'][0]['tray_type'] = 'PETG'
        rig.launch(); rig.seed()
        settings = {k:v for k,v in rig.api('/api/printers/p1').items()
                    if k not in ('id','status','machine','configuration_error')}
        requests = len(rig.broker.requests)
        rig.api('/api/printers/p1', dict(settings, nozzle_material='stainless_steel'), 'PUT')
        until(lambda: len(rig.broker.requests) > requests); rig.idle()
        gf = rig.api('/api/filaments', dict(name='PETG-GF 白', vendor='Fixture',
            material='PETG-GF', color='FFFFFFFF', bambu_filament_id=None), expected=201)
        data = dict(machine_profile_key=MACHINE, base_profile_key='Generic PETG',
                    overrides_json=dict(nozzle_temperature_initial_layer=250, nozzle_temperature=240))
        saved = rig.api(f'/api/filaments/{gf["id"]}/settings', data, expected=201)
        # Profile eligibility and temperature validation still apply.
        for machine in ['Bambu Lab P1S 0.2 nozzle', 'Bambu Lab A1 mini 0.2 nozzle']:
            rig.api(f'/api/filaments/{gf["id"]}/settings', dict(data, machine_profile_key=machine), expected=400)
        rig.api(f'/api/filaments/{gf["id"]}/settings/{saved["id"]}',
                dict(data, overrides_json={'nozzle_temperature':999}), 'PUT', expected=400)
        assert rig.api(f'/api/filaments/{gf["id"]}')['settings'][0]['overrides_json'] == data['overrides_json']
        slot = next(s for s in rig.api('/api/printers/p1/ams')['slots'] if s['slot_index'] == 0)
        rig.api(f'/api/printers/p1/ams/{slot["id"]}', dict(revision=slot['revision'], filament_id=gf['id']), 'PUT', 204)
        spec = rig.specification(0, gf['id']); rig.configure(spec)
        def admission():
            return rig.api('/api/queue?printer_id=p1&plate_id='+rig.plate['id'])['admission']
        assert admission()['allowed'], admission()
        # Plate requirements cannot substitute a different machine/nozzle profile.
        for machine in ['Bambu Lab P1S 0.2 nozzle', 'Bambu Lab A1 mini 0.2 nozzle']:
            conditions = dict(rig.plate['conditions'], required_machine_profile_key=machine)
            rig.api('/api/plates/'+rig.plate['id'], dict(name=rig.plate['name'],
                version=rig.plate['version'], models=rig.plate['models'], conditions=conditions), 'PUT', expected=409)
            assert rig.api('/api/plates/'+rig.plate['id']) == rig.plate
        job = rig.send(rig.add_action(spec))['waiting'][-1]
        # An observed mismatch with the registered nozzle still holds an existing job.
        for field, value in [('nozzle_diameter','0.2'), ('nozzle_type','hardened_steel')]:
            report = copy.deepcopy(rig.full); report['print'][field] = value
            rig.broker.send(report)
            status_key = 'nozzle_material' if field == 'nozzle_type' else field
            until(lambda: rig.api()['printer'][status_key] == value)
            assert not admission()['allowed']
            rig.next(job, expected=409)
            assert not rig.broker.prints and not rig.ftp.uploads
            rig.idle()
            until(lambda: rig.api()['printer'][status_key] == rig.full['print'][field])
        assert admission()['allowed']
        assert rig.api('/api/printers/p1')['nozzle_material'] == 'stainless_steel'
        # Queuing does not start a print; the manual Next and its retry send only once.
        assert not rig.broker.prints and not rig.ftp.uploads
        command = rig.command(dict(type='next', expected_job=job['id'], removed_job=None, cleared=True))
        rig.api(body=command)
        try:
            until(lambda: len(rig.broker.prints) == 1)
        except AssertionError:
            raise AssertionError(rig.api()['current']) from None
        rig.api(body=command)
        assert len(rig.broker.prints) == len(rig.ftp.uploads) == 1
        assert rig.broker.prints[0]['ams_mapping'] == [0]
        resolved = rig.traces()[0]['profiles']['filament']
        assert resolved['nozzle_temperature'] == ['240']
        assert resolved['nozzle_temperature_initial_layer'] == ['250']
        assert resolved['required_nozzle_HRC'] == ['60']
        rig.report('RUNNING'); rig.phase('printing')
        rig.report('FINISH'); rig.phase('awaiting_removal')
        assert len(rig.broker.prints) == 1 and not rig.ftp.errors
        result = dict(material='PETG-GF', nozzle='stainless_steel', diameter='0.4',
                      profile_hrc=60, temperature=240, wrong_profiles_rejected=True,
                      invalid_temperature_rejected=True, observed_mismatch_rejected=True,
                      manual_next_once=True, awaiting_removal=True)
        (rig.output/'verified.json').write_text(json.dumps(result, indent=2))
        print(json.dumps(result))
    finally:
        rig.close()


if __name__ == '__main__':
    main()
