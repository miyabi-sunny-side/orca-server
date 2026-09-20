"""Verify a local or published image: python3 tests/container.py IMAGE OUTPUT_DIR."""
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
import zipfile
from slicer_cli import boxes, REPO


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()


def main():
    image = sys.argv[1]
    output = Path(sys.argv[2]).resolve(); output.mkdir(parents=True, exist_ok=True)
    name = 'orca-check-' + uuid.uuid4().hex
    volume = name + '-plates'
    docker('volume', 'create', volume)
    with tempfile.TemporaryDirectory(prefix='orca-container-') as directory:
        cert = Path(directory) / 'cert.pem'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:P-256',
                        '-nodes', '-days', '1', '-subj', '/CN=isolated-printer', '-keyout', directory+'/key',
                        '-out', str(cert)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        base = ''
        def request(path, data=None, headers=None):
            req = urllib.request.Request(base+path, data=data, headers=headers or {})
            with urllib.request.urlopen(req, timeout=90) as response: return response.read()
        def api(path, body=None):
            return json.loads(request(path, None if body is None else json.dumps(body).encode(), {'Content-Type':'application/json'}))
        def start():
            nonlocal base
            docker('run', '-d', '--name', name, '-p', '127.0.0.1::3000', '-v', volume+':/data/plates',
                   '-v', str(cert)+':/config/cert.pem:ro', '-e', 'P1_IP=127.0.0.1', '-e', 'P1_SERIAL=ISOLATED',
                   '-e', 'P1_ACCESS_CODE=TESTONLY', '-e', 'P1_TLS_CERT=/config/cert.pem', '-e', 'P1_MQTT_PORT=9', image)
            port = docker('port', name, '3000/tcp').rsplit(':', 1)[1]; base = 'http://127.0.0.1:'+port
            for _ in range(200):
                try:
                    if request('/healthz') == b'ok\n': return
                except (OSError, urllib.error.HTTPError): pass
                if docker('inspect', '--format', '{{.State.Running}}', name) != 'true':
                    raise RuntimeError(docker('logs', name))
                time.sleep(.1)
            raise RuntimeError('Web did not start')
        try:
            start()
            assert docker('exec', name, 'id', '-u') == '10001'
            environment = docker('exec', name, 'env').splitlines()
            assert not any(e.startswith(('DISPLAY=', 'WAYLAND_DISPLAY=')) for e in environment)
            assert api('/api/printer/status')['connection'] == 'disconnected'
            assert api('/api/slicer/profiles')['version'] == '2.4.2'
            assert b'<!doctype html' in request('/').lower()
            cube = (REPO/'tests/fixtures/cube.stl').read_bytes()
            fields = [('name', None, b'Two cubes'), ('models','a.stl',cube), ('models','b.stl',cube)]
            body = b''
            for field, filename, value in fields:
                header = f'Content-Disposition: form-data; name="{field}"'
                if filename: header += f'; filename="{filename}"'
                body += b'--orca-boundary\r\n'+header.encode()+b'\r\n\r\n'+value+b'\r\n'
            body += b'--orca-boundary--\r\n'
            plate = json.loads(request('/api/plates', body, {'Content-Type':'multipart/form-data; boundary=orca-boundary'}))
            plate = json.loads(request('/api/plates/'+plate['id']+'/slice', b''))
            data = {key: request('/api/plates/'+plate['id']+'/files/'+plate[key]) for key in ('project','print')}
            bounds = boxes(data['project']); printed = boxes(data['print'])
            assert len(bounds) == len(printed) == 2
            assert all(0 <= lo < hi <= limit for box in bounds for (lo,hi),limit in zip(box,[256,256,250]))
            assert any(bounds[0][a][1] <= bounds[1][a][0] or bounds[1][a][1] <= bounds[0][a][0] for a in (0,1))
            assert all(abs(x-y)<.001 for a,b in zip(bounds,printed) for c,d in zip(a,b) for x,y in zip(c,d))
            assert len(zipfile.ZipFile(io.BytesIO(data['print'])).read('Metadata/plate_1.gcode')) > 1000
            q = api('/api/queue')
            q = api('/api/queue', dict(generation=q['generation'],request_id=q['request_id'],action=dict(type='add',plate_id=plate['id'],revision=plate['revision'],ams_slot=0)))
            assert len(q['waiting']) == 1 and not q['allowed']['next']
            (output/'first.log').write_text(docker('logs', name))
            docker('rm', '-f', name)
            start()
            assert api('/api/plates/'+plate['id']) == plate
            for key in data:
                assert request('/api/plates/'+plate['id']+'/files/'+plate[key]) == data[key]
                (output/Path(plate[key]).name).write_bytes(data[key])
            q = api('/api/queue'); assert q['waiting'] == [] and q['current'] is None
            for _ in range(40):
                if docker('inspect','--format','{{.State.Health.Status}}',name) == 'healthy': break
                time.sleep(1)
            else: raise AssertionError('Docker health check did not pass')
            result = dict(image=image,uid=10001,version='2.4.2',headless=True,world_bounds=bounds,
                          persistent_plate_and_artifacts=True,recreated_queue_empty=True,disconnected_web_available=True,healthy=True)
            (output/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
        finally:
            try:
                log = docker('logs', name); (output/'server.log').write_text(log); assert 'TESTONLY' not in log
            finally:
                subprocess.run(['docker','rm','-f',name],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                docker('volume','rm',volume)


if __name__ == '__main__': main()
