"""Real OrcaSlicer integration: python3 tests/slicer_cli.py APPDIR OUTPUT_DIR."""
import concurrent.futures
import io
import json
import os
from pathlib import Path
import shlex
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import urllib.parse
import xml.etree.ElementTree as ET
import zipfile

REPO = Path(__file__).resolve().parents[1]
CORE = '{http://schemas.microsoft.com/3dmanufacturing/core/2015/02}'
PRODUCTION = '{http://schemas.microsoft.com/3dmanufacturing/production/2015/06}'


def boxes(data):
    """Compose component/build transforms before comparing world coordinates."""
    archive = zipfile.ZipFile(io.BytesIO(data))

    def transform(vertex, text):
        matrix = list(map(float, text.split())) if text else [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]
        return [sum(vertex[j] * matrix[j * 3 + i] for j in range(3)) + matrix[9 + i] for i in range(3)]

    def vertices(path, object_id):
        root = ET.fromstring(archive.read(path))
        obj = root.find(f'{CORE}resources/{CORE}object[@id="{object_id}"]')
        result = [[float(v.get(a)) for a in ('x', 'y', 'z')] for v in obj.findall(f'{CORE}mesh/{CORE}vertices/{CORE}vertex')]
        for component in obj.findall(f'{CORE}components/{CORE}component'):
            target = component.get(PRODUCTION + 'path', path).lstrip('/')
            result.extend(transform(v, component.get('transform')) for v in vertices(target, component.get('objectid')))
        return result

    root = ET.fromstring(archive.read('3D/3dmodel.model'))
    result = []
    for item in root.findall(f'{CORE}build/{CORE}item'):
        points = [transform(v, item.get('transform')) for v in vertices('3D/3dmodel.model', item.get('objectid'))]
        result.append([[min(v[a] for v in points), max(v[a] for v in points)] for a in range(3)])
    return sorted(result)


def main():
    appdir = Path(sys.argv[1]).resolve()
    evidence = Path(sys.argv[2]).resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='orca-cli-check-') as tmp:
        tmp = Path(tmp)
        wrapper = tmp / 'app'
        wrapper.mkdir()
        (wrapper / 'resources').symlink_to(appdir / 'resources', target_is_directory=True)
        mode = tmp / 'mode'
        pid = tmp / 'child.pid'
        mode.write_text('normal')
        # The production command still executes the official binary. These two
        # deliberate faults test our timeout/exit handling without a printer.
        (wrapper / 'AppRun').write_text(
            '#!/bin/sh\n' + f'mode=$(cat {shlex.quote(str(mode))})\n'
            + 'case "$mode" in\nfail) exit 7;;\nsleep) '
            + f'echo $$ > {shlex.quote(str(pid))}; exec sleep 30;;\nesac\n'
            + f'exec {shlex.quote(str(appdir / "AppRun"))} "$@"\n')
        (wrapper / 'AppRun').chmod(0o700)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        base = f'http://127.0.0.1:{port}'
        env = dict(os.environ, PORT=str(port), PLATES_DIR=str(tmp / 'plates'), ORCA_APPDIR=str(wrapper), ORCA_TIMEOUT_SECS='2')
        env.pop('SCAD_LIVE_URL', None)
        env.pop('DISPLAY', None)
        env.pop('WAYLAND_DISPLAY', None)
        log = (evidence / 'server.log').open('w')
        server = subprocess.Popen([str(REPO / 'target/debug/orca-server')], cwd=tmp, env=env, stdout=log, stderr=log)

        def request(path, data=None, headers=None):
            req = urllib.request.Request(base + path, data=data, headers=headers or {})
            try:
                with urllib.request.urlopen(req, timeout=20) as response:
                    return response.status, response.read()
            except urllib.error.HTTPError as error:
                return error.code, error.read()

        def json_request(path, data=None, headers=None):
            status, body = request(path, data, headers)
            return status, json.loads(body)

        def save(name, models, settings=None):
            fields = [('name', None, name.encode()), ('settings', None, json.dumps(settings or {}).encode())]
            fields += [("models", f'{index}.stl', model) for index, model in enumerate(models)]
            body = b''
            for field, filename, data in fields:
                disposition = f'Content-Disposition: form-data; name="{field}"'
                if filename:
                    disposition += f'; filename="{filename}"'
                body += b'--orca-boundary\r\n' + disposition.encode() + b'\r\n\r\n' + data + b'\r\n'
            body += b'--orca-boundary--\r\n'
            status, plate = json_request('/api/plates', body, {'Content-Type': 'multipart/form-data; boundary=orca-boundary'})
            assert status == 201, (status, plate)
            return plate

        def slice_plate(plate):
            return json_request(f'/api/plates/{plate["id"]}/slice', b'')

        try:
            for _ in range(100):
                if server.poll() is not None:
                    raise RuntimeError('Server exited; see server.log')
                try:
                    if request('/healthz')[0] == 200:
                        break
                except OSError:
                    pass
                time.sleep(.1)
            else:
                raise RuntimeError('Server did not start')
            status, choices = json_request('/api/slicer/profiles')
            assert status == 200 and choices['version'] == '2.4.2'
            cube = (REPO / 'tests/fixtures/cube.stl').read_bytes()
            saved = save('Two cubes', [cube, cube])
            status, sliced = slice_plate(saved)
            assert status == 200, (status, sliced)
            downloaded = {}
            for key in ('project', 'print'):
                status, data = request(f'/api/plates/{saved["id"]}/files/{sliced[key]}')
                assert status == 200
                (evidence / Path(sliced[key]).name).write_bytes(data)
                downloaded[key] = data
            before, after = boxes(downloaded['project']), boxes(downloaded['print'])
            assert len(before) == len(after) == 2
            status, preview = json_request('/api/plates/' + saved['id'] + '/layout')
            assert status == 200 and preview['revision'] == sliced['revision'], (status, preview)
            assert sorted(item['bounds'] for item in preview['models']) == before
            assert sorted(item['index'] for item in preview['models']) == [0, 1]
            for bounds in before:
                assert all(0 <= low < high <= limit for (low, high), limit in zip(bounds, [256, 256, 250])), bounds
            assert any(before[0][axis][1] <= before[1][axis][0] or before[1][axis][1] <= before[0][axis][0] for axis in (0, 1)), before
            assert all(abs(x-y) < .001 for a, b in zip(before, after) for c, d in zip(a, b) for x, y in zip(c, d)), (before, after)
            z = zipfile.ZipFile(io.BytesIO(downloaded['print']))
            assert len(z.read('Metadata/plate_1.gcode')) > 1000
            assert not any(n.endswith('.gcode') for n in zipfile.ZipFile(io.BytesIO(downloaded['project'])).namelist())
            selection = {'process': '0.16mm Optimal @BBL X1C', 'filament': 'Bambu PLA Basic @BBL X1C', 'bed': 'High Temp Plate'}
            selected = save('Other settings', [cube], {'slicer': selection})
            status, selected = slice_plate(selected)
            assert status == 200 and selected['settings']['slicer'] == dict(selection, machine=choices['printer']), (status, selected)
            for key in ('project', 'print'):
                data = request(f'/api/plates/{selected["id"]}/files/{selected[key]}')[1]
                settings = json.loads(zipfile.ZipFile(io.BytesIO(data)).read('Metadata/project_settings.config'))
                assert settings['print_settings_id'] == selection['process']
                assert settings['filament_settings_id'] == [selection['filament']]
                assert settings['curr_bed_type'] == selection['bed']
            a1 = 'Bambu Lab A1 mini 0.2 nozzle'
            status, a1_choices = json_request('/api/slicer/profiles?machine=' + urllib.parse.quote(a1))
            assert status == 200
            a1_plate = save('A1 mini fine nozzle', [cube], {'slicer': a1_choices['defaults']})
            status, a1_plate = slice_plate(a1_plate)
            assert status == 200, (status, a1_plate)
            status, preview = json_request('/api/plates/' + a1_plate['id'] + '/layout')
            assert status == 200 and preview['bed'] == [[0, 180], [0, 180]], preview
            for key in ('project', 'print'):
                data = request(f'/api/plates/{a1_plate["id"]}/files/{a1_plate[key]}')[1]
                settings = json.loads(zipfile.ZipFile(io.BytesIO(data)).read('Metadata/project_settings.config'))
                assert settings['printer_settings_id'] == a1
                assert settings['nozzle_diameter'] == ['0.2']
                assert settings['print_settings_id'] == a1_choices['defaults']['process']
            # Two 200 mm cubes require multiple plates; a 400 mm cube cannot fit.
            for size, count in [(200, 2), (400, 1)]:
                scaled = bytearray(cube)
                for triangle in range(struct.unpack_from('<I', cube, 80)[0]):
                    for offset in range(96 + triangle * 50, 132 + triangle * 50, 4):
                        coordinate = struct.unpack_from('<f', cube, offset)[0]
                        struct.pack_into('<f', scaled, offset, coordinate * size / 20)
                impossible = save('Does not fit', [scaled] * count)
                status, error = slice_plate(impossible)
                assert status in (400, 502), (size, status, error)
                if size == 200:
                    assert status == 400 and 'fit together' in error['error'], (status, error)
                assert json_request('/api/plates/' + impossible['id'])[1] == impossible
            invalid = save('Non-solid triangle', [(REPO / 'tests/fixtures/triangle.stl').read_bytes()])
            assert slice_plate(invalid)[0] in (400, 502)
            mode.write_text('fail')
            assert slice_plate(sliced)[0] == 502
            assert json_request('/api/plates/' + sliced['id'])[1] == sliced
            mode.write_text('sleep')
            with concurrent.futures.ThreadPoolExecutor() as executor:
                pending = executor.submit(slice_plate, sliced)
                for _ in range(100):
                    if pid.exists():
                        break
                    time.sleep(.01)
                assert pid.exists()
                assert slice_plate(sliced)[0] == 409
                assert request('/healthz') == (200, b'ok\n')
                assert pending.result()[0] == 504
            assert not Path('/proc/' + pid.read_text().strip()).exists(), 'timed-out child is still alive'
            assert json_request('/api/plates/' + sliced['id'])[1] == sliced
            mode.write_text('normal')
            assert slice_plate(sliced)[0] == 200, 'slot not released after failure'
            assert json_request(f'/api/plates/{sliced["id"]}/slice', b'', {'Origin': 'https://untrusted.invalid'})[0] == 403
            results = {'version': choices['version'], 'world_bounds': before, 'roundtrip_bounds': after, 'nondefault_settings': selection, 'headless': True, 'saved_project_only_resliced': True, 'multiple_plate_rejected': True, 'oversized_rejected': True, 'invalid_model_rejected': True, 'failure_preserves_plate': True, 'timeout_preserves_plate': True, 'timeout_child_reaped': True, 'busy_rejected': True, 'server_stays_alive': True}
            results['a1_mini_02_profile_and_bed'] = True
            (evidence / 'result.json').write_text(json.dumps(results, indent=2))
            print(json.dumps(results))
        finally:
            server.terminate()
            server.wait(timeout=10)
            log.close()


if __name__ == '__main__':
    main()
