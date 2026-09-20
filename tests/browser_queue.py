"""Chromium + real queue API + isolated P1: python3 tests/browser_queue.py BINARY OUTPUT_DIR."""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import uuid
from printer_mqtt import SERIAL, SECRET, until
from printer_start import ARTIFACT, PrintBroker, Ftps, REPO


def main():
    binary = str(Path(sys.argv[1]).resolve())
    output = Path(sys.argv[2]).resolve(); output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='orca-queue-browser-') as directory:
        tmp = Path(directory)
        subprocess.run(['openssl','req','-x509','-newkey','ec','-pkeyopt','ec_paramgen_curve:P-256','-nodes','-days','1',
                        '-subj','/CN=isolated-printer','-keyout',str(tmp/'key'),'-out',str(tmp/'cert')],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        broker = PrintBroker(tmp/'cert', tmp/'key'); ftp = Ftps(tmp/'cert', tmp/'key')
        store = tmp/'plates'
        for name in ['A · 机の配線整理・ケーブルホルダー', 'B · 小物ケース', 'C · 取り付けパーツ', 'D · 予備のパーツ']:
            plate_id, revision = str(uuid.uuid4()), str(uuid.uuid4())
            data = store/plate_id/'revisions'/revision; data.mkdir(parents=True)
            for i in range(2): (data/f'{i}.stl').write_bytes((REPO/'tests/fixtures/cube.stl').read_bytes())
            for filename in ['project.3mf', 'print.gcode.3mf']: (data/filename).write_bytes(ARTIFACT)
            plate = dict(format_version=1, id=plate_id, revision=revision, name=name, settings={},
                         models=[dict(name=f'cube-{i}.stl', path=f'revisions/{revision}/{i}.stl', source=None) for i in range(2)],
                         project=f'revisions/{revision}/project.3mf', print=f'revisions/{revision}/print.gcode.3mf')
            (store/plate_id/'plate.json').write_text(json.dumps(plate))
        full = json.loads((REPO/'tests/fixtures/p1_status.json').read_text())
        full['print']['ams']['tray_exist_bits'] = '9'
        full['print']['ams']['ams'][0]['tray'][3].update(tray_type='PLA', tray_color='00FFFFFF')

        class Control(BaseHTTPRequestHandler):
            def do_GET(self): self.reply()
            def do_POST(self):
                value = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if value.get('revise_plate'):
                    metadata = store/value['revise_plate']/'plate.json'
                    plate = json.loads(metadata.read_text()); old = plate['revision']; new = str(uuid.uuid4())
                    (metadata.parent/'revisions'/old).rename(metadata.parent/'revisions'/new)
                    plate.update(revision=new, name=plate['name']+'（更新）')
                    for model in plate['models']: model['path'] = model['path'].replace(old, new)
                    for field in ['project', 'print']: plate[field] = plate[field].replace(old, new)
                    metadata.write_text(json.dumps(plate))
                elif value.get('fail_upload'): ftp.actions.put('fail')
                else:
                    report = copy.deepcopy(full)
                    if value.get('state'):
                        command = broker.prints[value.get('index', -1)]
                        report['print'].update(gcode_state=value['state'], subtask_name=command['subtask_name'], gcode_file=command['file'])
                    broker.send(report)
                self.reply()
            def reply(self):
                data = json.dumps(dict(prints=broker.prints, uploads=ftp.uploads)).encode()
                self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
            def log_message(self, *_): pass
        control = ThreadingHTTPServer(('127.0.0.1',0), Control)
        threading.Thread(target=control.serve_forever, daemon=True).start()
        with socket.socket() as sock: sock.bind(('127.0.0.1',0)); port = sock.getsockname()[1]
        env = {k:v for k,v in os.environ.items() if not k.startswith('P1_') and k not in ('ORCA_APPDIR','SCAD_LIVE_URL')}
        env.update(PORT=str(port), PLATES_DIR=str(store), LOG_LEVEL='trace', P1_IP='127.0.0.1', P1_SERIAL=SERIAL,
                   P1_ACCESS_CODE=SECRET, P1_TLS_CERT=str(tmp/'cert'), P1_MQTT_PORT=str(broker.port), P1_FTPS_PORT=str(ftp.port),
                   P1_START_TIMEOUT_SECS='60', E2E_BASE_URL=f'http://127.0.0.1:{port}', E2E_EVIDENCE_DIR=str(output),
                   E2E_PRINTER_CONTROL=f'http://127.0.0.1:{control.server_port}')
        with (output/'server.log').open('w') as log:
            process = subprocess.Popen([binary], cwd=tmp, env=env, stdout=log, stderr=log)
            try:
                until(lambda: len(broker.requests) == 1); broker.send(full)
                subprocess.run(['npm','run','test:e2e','--','queue-live.spec.ts','--workers=1'],cwd=REPO/'client',env=env,check=True)
            finally:
                process.terminate(); process.wait(timeout=5)
                control.shutdown(); control.server_close(); ftp.close(); broker.close()
        assert SECRET.encode() not in (output/'server.log').read_bytes()
        (output/'protocol-result.json').write_text(json.dumps(dict(prints=len(broker.prints), uploads=len(ftp.uploads), verified_saved_bytes=True, secrets_redacted=True),indent=2))


if __name__ == '__main__': main()
