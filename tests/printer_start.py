"""Observe real FTPS/TLS and MQTT for a saved Orca 2.4.2 cube print.

python3 tests/printer_start.py BINARY OUTPUT_DIR
Only disposable loopback peers are used; no hardware print is started.
"""
import copy
import json
import os
from pathlib import Path
import queue
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid
from printer_mqtt import Broker, SERIAL, SECRET, until

REPO = Path(__file__).resolve().parents[1]
ARTIFACT = (REPO / 'tests/fixtures/p1_print.gcode.3mf').read_bytes()


class PrintBroker(Broker):
    def __init__(self, cert, key):
        self.prints = []
        super().__init__(cert, key)

    def on_request(self, value):
        if 'pushing' in value:
            super().on_request(value)
        else:
            assert value['print']['command'] == 'project_file'
            self.prints.append(value['print'])


class Ftps:
    def __init__(self, cert, key):
        self.context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.context.maximum_version = ssl.TLSVersion.TLSv1_2
        self.context.load_cert_chain(cert, key)
        self.socket = socket.create_server(('127.0.0.1', 0))
        self.socket.settimeout(.1)
        self.port = self.socket.getsockname()[1]
        self.actions = queue.Queue()
        self.uploads = []
        self.errors = []
        self.tls_failures = 0
        self.gate = threading.Event()
        self.received = threading.Event()
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stopped.is_set():
            try:
                raw, _ = self.socket.accept()
            except socket.timeout:
                continue
            raw.settimeout(5)
            try:
                with self.context.wrap_socket(raw, server_side=True) as peer:
                    self.session(peer)
            except ssl.SSLError:
                self.tls_failures += 1
            except (EOFError, ConnectionError, socket.timeout):
                pass
            except Exception as error:
                self.errors.append(repr(error))
            finally:
                raw.close()

    def session(self, peer):
        with peer.makefile('rb') as reader:
            peer.sendall(b'220 isolated peer ready\r\n')
            data_listener = None
            try:
                while not self.stopped.is_set():
                    line = reader.readline(4096).decode().rstrip('\r\n')
                    if not line:
                        return
                    command, _, arg = line.partition(' ')
                    if command == 'USER':
                        assert arg == 'bblp'
                        peer.sendall(b'331 password required\r\n')
                    elif command == 'PASS':
                        assert arg == SECRET
                        peer.sendall(b'230 logged in\r\n')
                    elif command in ('PBSZ', 'PROT', 'TYPE'):
                        assert arg == {'PBSZ':'0', 'PROT':'P', 'TYPE':'I'}[command]
                        peer.sendall(b'200 ok\r\n')
                    elif command == 'PASV':
                        data_listener = socket.create_server(('127.0.0.1', 0))
                        data_listener.settimeout(5)
                        port = data_listener.getsockname()[1]
                        # The application must ignore PASV's foreign IP, while using its dynamic port.
                        peer.sendall(f'227 Passive (203,0,113,22,{port//256},{port%256})\r\n'.encode())
                    elif command == 'STOR':
                        assert arg.startswith('orca-') and arg.endswith('.gcode.3mf') and '/' not in arg
                        peer.sendall(b'150 send data\r\n')
                        raw, _ = data_listener.accept()
                        raw.settimeout(5)
                        with self.context.wrap_socket(raw, server_side=True) as data:
                            assert data.session_reused, 'P1 data connection must reuse the control TLS session'
                            content = b''
                            while True:
                                chunk = data.recv(65536)
                                if not chunk:
                                    break
                                content += chunk
                        assert content == ARTIFACT, 'uploaded bytes differ from the saved print'
                        self.uploads.append(arg)
                        try:
                            action = self.actions.get_nowait()
                        except queue.Empty:
                            action = 'ok'
                        if action == 'wait':
                            self.received.set()
                            assert self.gate.wait(10)
                        if action == 'fail':
                            peer.sendall(('550 ' + SECRET + '\r\n').encode())
                        else:
                            peer.sendall(b'226 transfer complete\r\n')
                    elif command == 'QUIT':
                        peer.sendall(b'221 bye\r\n')
                        return
                    else:
                        raise AssertionError(f'unexpected command {command}')
            finally:
                if data_listener:
                    data_listener.close()

    def close(self):
        self.stopped.set()
        self.gate.set()
        self.thread.join(timeout=6)
        self.socket.close()
        assert not self.thread.is_alive()
        assert not self.errors, self.errors


def main():
    binary = str(Path(sys.argv[1]).resolve())
    output = Path(sys.argv[2]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='orca-start-') as directory:
        tmp = Path(directory)
        for name in ['trusted', 'other']:
            subprocess.run(['openssl','req','-x509','-newkey','ec','-pkeyopt','ec_paramgen_curve:P-256','-nodes','-days','1',
                            '-subj','/CN=isolated-printer','-keyout',str(tmp/f'{name}.key'),'-out',str(tmp/f'{name}.pem')],
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        plate_id, revision = str(uuid.uuid4()), str(uuid.uuid4())
        store = tmp/'plates'
        artifacts = store/plate_id/'revisions'/revision
        artifacts.mkdir(parents=True)
        (artifacts/'print.gcode.3mf').write_bytes(ARTIFACT)
        for index in range(2):
            (artifacts/f'{index}.stl').write_bytes((REPO/'tests/fixtures/cube.stl').read_bytes())
        plate = dict(format_version=1,id=plate_id,revision=revision,name='Two cubes',settings={},
                     models=[dict(name=f'cube-{i}.stl',path=f'revisions/{revision}/{i}.stl',source=None) for i in range(2)],
                     project=None,print=f'revisions/{revision}/print.gcode.3mf')
        (store/plate_id/'plate.json').write_text(json.dumps(plate))
        full = json.loads((REPO/'tests/fixtures/p1_status.json').read_text())
        # A nonzero tray proves that physical tray selection is not the slicer's material index.
        full['print']['ams']['tray_exist_bits'] = '9'
        full['print']['ams']['ams'][0]['tray'][3].update(tray_type='PLA',tray_color='00FFFFFF')
        broker = PrintBroker(tmp/'trusted.pem',tmp/'trusted.key')
        ftp = Ftps(tmp/'trusted.pem',tmp/'trusted.key')
        wrong = Ftps(tmp/'other.pem',tmp/'other.key')
        with socket.socket() as s:
            s.bind(('127.0.0.1',0)); port = s.getsockname()[1]
        env = {k:v for k,v in os.environ.items() if not k.startswith('P1_') and k not in ('ORCA_APPDIR','SCAD_LIVE_URL')}
        env.update(PORT=str(port),PLATES_DIR=str(store),LOG_LEVEL='trace',P1_IP='127.0.0.1',P1_SERIAL=SERIAL,
                   P1_ACCESS_CODE=SECRET,P1_TLS_CERT=str(tmp/'trusted.pem'),P1_MQTT_PORT=str(broker.port),
                   P1_FTPS_PORT=str(ftp.port),P1_START_TIMEOUT_SECS='3')
        log = (output/'server.log').open('w')
        process = subprocess.Popen([binary],env=env,cwd=tmp,stdout=log,stderr=log)
        def api(path='/api/printer/status', body=None, origin=None, expected=200):
            headers = {'Content-Type':'application/json'}
            if origin: headers['Origin'] = origin
            request = urllib.request.Request(f'http://127.0.0.1:{port}'+path,data=json.dumps(body).encode() if body is not None else None,headers=headers)
            try:
                response = urllib.request.urlopen(request, timeout=3)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                raw = response.read()
                assert SECRET.encode() not in raw
                assert response.code == expected, (path,response.code,raw)
                return json.loads(raw)
        def start(slot=3, expected=202, origin=None, rev=revision):
            return api(f'/api/plates/{plate_id}/print',dict(revision=rev,ams_slot=slot),origin,expected)
        def phase(expected):
            return until(lambda: api()['start'] and api()['start']['phase'] == expected)
        def idle():
            broker.send(full)
            until(lambda: api()['ready_to_print'])
        try:
            until(lambda: len(broker.requests) == 1)
            start(expected=409) # not synchronized
            idle()
            start(expected=403,origin='https://foreign.invalid')
            start(expected=409,rev=str(uuid.uuid4()))
            for slot in [1,2,16,255]: start(slot,409)
            busy = copy.deepcopy(full); busy['print']['gcode_state'] = 'RUNNING'
            broker.send(busy); until(lambda: not api()['ready_to_print']); start(expected=409)
            idle()
            bad = copy.deepcopy(full); bad['print']['print_error'] = 42
            broker.send(bad); until(lambda: not api()['ready_to_print']); start(expected=409)
            idle()
            attempt = start()
            start(expected=409) # concurrent request cannot produce a second upload/command
            until(lambda: len(broker.prints) == 1)
            command = broker.prints[0]
            assert command['url'] == 'ftp:///' + ftp.uploads[0]
            assert command['param'] == 'Metadata/plate_1.gcode' and command['ams_mapping'] == [3] and command['use_ams']
            phase('awaiting_confirmation')
            broker.send({'print':dict(command='project_file',sequence_id='wrong',result='success')})
            time.sleep(.15); assert api()['start']['phase'] == 'awaiting_confirmation'
            broker.send({'print':dict(command='project_file',sequence_id=command['sequence_id'],result='success')})
            phase('accepted')
            running = copy.deepcopy(full)
            running['print'].update(gcode_state='RUNNING',subtask_name=command['subtask_name'],gcode_file=command['file'])
            broker.send(running); phase('printing')
            broker.actions.put('disconnect'); phase('unknown')
            until(lambda: len(broker.requests) == 2)
            time.sleep(.2); assert len(broker.prints) == 1
            broker.send(running); phase('printing')
            running['print']['gcode_state'] = 'FINISH'; broker.send(running); phase('finished')
            idle()
            ftp.actions.put('fail')
            start(); phase('upload_failed'); assert len(broker.prints) == 1
            ftp.actions.put('wait')
            start(); assert ftp.received.wait(5)
            broker.send(busy); until(lambda: not api()['ready_to_print'])
            ftp.gate.set(); phase('not_sent'); assert len(broker.prints) == 1
            idle()
            start(); until(lambda: len(broker.prints) == 2)
            rejected = broker.prints[-1]
            broker.send({'print':dict(command='project_file',sequence_id=rejected['sequence_id'],result='fail')})
            phase('rejected')
            unknown = start(); until(lambda: len(broker.prints) == 3)
            phase('unknown'); start(expected=409)
            until(lambda: len(broker.requests) == 3)
            assert len(broker.prints) == 3
            idle()
            path = f"/api/printer/start/{unknown['id']}/resolve"
            api(path,dict(checked_printer=False),expected=400)
            api(path,dict(checked_printer=True),origin='https://foreign.invalid',expected=403)
            api(path,dict(checked_printer=True))
            phase('resolved')
        finally:
            process.terminate(); process.wait(timeout=5); log.close()
        # Keep MQTT trusted while a separate FTPS endpoint presents an untrusted certificate.
        env['P1_FTPS_PORT'] = str(wrong.port)
        log = (output/'wrong-cert.log').open('w')
        process = subprocess.Popen([binary],env=env,cwd=tmp,stdout=log,stderr=log)
        try:
            until(lambda: len(broker.requests) == 4)
            idle(); start(); phase('upload_failed')
            assert wrong.tls_failures and not wrong.uploads and len(broker.prints) == 3
        finally:
            process.terminate(); process.wait(timeout=5); log.close()
            ftp.close(); wrong.close(); broker.close()
        for path in list(output.glob('*.log')) + list(store.rglob('*')):
            if path.is_file(): assert SECRET.encode() not in path.read_bytes()
    result = dict(saved_bytes_uploaded=True, implicit_tls_session_reused=True, passive_port_observed=True,
                  selected_ams_mapping=True, unknown_busy_error_empty_or_wrong_material_refused=True,
                  concurrent_start_refused=True, upload_failure_no_command=True, state_change_during_upload_no_command=True,
                  ack_distinct_from_printing=True, matching_status_required=True, timeout_and_reconnect_no_replay=True,
                  explicit_resolution=True, wrong_ftp_certificate_refused=True, browser_origin_checked=True, secrets_redacted=True)
    (output/'result.json').write_text(json.dumps(result,indent=2))
    print(json.dumps(result))


if __name__ == '__main__':
    main()
