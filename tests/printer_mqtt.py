"""Isolated MQTT/TLS integration: python3 tests/printer_mqtt.py BINARY OUTPUT_DIR.

Requires Python 3 and openssl. Uses a disposable TLS peer and app-owned fixtures;
no printer, network credentials, or third-party MQTT service is contacted.
"""
import copy
import json
import os
from pathlib import Path
import queue
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

REPO = Path(__file__).resolve().parents[1]
SERIAL = 'TESTP1SERIAL'
SECRET = 'isolated-test-access-code'
REPORT = f'device/{SERIAL}/report'


def packet(header, body):
    length = len(body)
    result = bytes([header])
    while True:
        part = length % 128
        length //= 128
        result += bytes([part | (128 if length else 0)])
        if not length:
            return result + body


def read_packet(peer):
    first = peer.recv(1)
    if not first:
        raise EOFError
    peer.settimeout(2)
    try:
        length, shift = 0, 0
        while True:
            digit = peer.recv(1)
            if not digit:
                raise EOFError
            length |= (digit[0] & 127) << shift
            if not digit[0] & 128:
                break
            shift += 7
            assert shift <= 21
        assert length <= 1024 * 1024
        body = b''
        while len(body) < length:
            chunk = peer.recv(length - len(body))
            if not chunk:
                raise EOFError
            body += chunk
        return first[0], body
    finally:
        peer.settimeout(.1)


class Broker:
    def __init__(self, cert, key):
        self.context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.context.maximum_version = ssl.TLSVersion.TLSv1_2
        self.context.load_cert_chain(cert, key)
        self.socket = socket.create_server(('127.0.0.1', 0))
        self.socket.settimeout(.1)
        self.port = self.socket.getsockname()[1]
        self.actions = queue.Queue()
        self.requests = []
        self.errors = []
        self.tls_failures = 0
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stopped.is_set():
            try:
                raw, _ = self.socket.accept()
            except socket.timeout:
                continue
            raw.settimeout(2)
            try:
                with self.context.wrap_socket(raw, server_side=True) as peer:
                    self.session(peer)
            except ssl.SSLError:
                self.tls_failures += 1
            except (EOFError, ConnectionError, socket.timeout):
                pass
            except Exception as error:
                self.errors.append(type(error).__name__)
            finally:
                raw.close()

    def session(self, peer):
        header, login = read_packet(peer)
        assert header == 0x10 and b'bblp' in login and SECRET.encode() in login
        assert login[7] & 2, 'clean session required'
        peer.sendall(b'\x20\x02\x00\x00')
        header, body = read_packet(peer)
        assert header == 0x82 and REPORT.encode() in body
        peer.sendall(b'\x90\x03' + body[:2] + b'\x00')
        while not self.stopped.is_set():
            try:
                action = self.actions.get_nowait()
            except queue.Empty:
                action = None
            if action == 'disconnect':
                return
            if action == 'invalid-login-packet':
                peer.sendall(packet(0x10, login))
            elif isinstance(action, tuple):
                value, retained, topic = action
                payload = value if isinstance(value, bytes) else json.dumps(value).encode()
                encoded = topic.encode()
                peer.sendall(packet(0x31 if retained else 0x30, struct.pack('!H', len(encoded)) + encoded + payload))
            try:
                header, body = read_packet(peer)
            except socket.timeout:
                continue
            if header == 0xc0:
                peer.sendall(b'\xd0\x00')
            else:
                assert header == 0x30, 'only non-retained QoS0 status requests are allowed'
                length = struct.unpack('!H', body[:2])[0]
                assert body[2:2+length].decode() == f'device/{SERIAL}/request'
                value = json.loads(body[2+length:])
                assert value['pushing']['command'] == 'pushall'
                assert value['pushing']['version'] == 1 and value['pushing']['push_target'] == 1
                self.requests.append(value)

    def send(self, value, retained=False, topic=REPORT):
        self.actions.put((value, retained, topic))

    def close(self):
        self.stopped.set()
        self.thread.join(timeout=5)
        self.socket.close()
        assert not self.thread.is_alive()
        assert not self.errors, self.errors


def until(check, seconds=12):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (OSError, ValueError):
            pass
        time.sleep(.05)
    raise AssertionError('condition did not become true')


def main():
    binary = Path(sys.argv[1]).resolve()
    output = Path(sys.argv[2]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    full = json.loads((REPO / 'tests/fixtures/p1_status.json').read_text())
    # Unknown fields, even if a peer echoes a credential, must not be exposed.
    full['print']['access_code'] = SECRET
    with tempfile.TemporaryDirectory(prefix='orca-mqtt-') as directory:
        tmp = Path(directory)
        for name in ['trusted', 'other']:
            subprocess.run(['openssl', 'req', '-x509', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:P-256',
                            '-nodes', '-days', '1', '-subj', '/CN=isolated-printer',
                            '-keyout', str(tmp / f'{name}.key'), '-out', str(tmp / f'{name}.pem')],
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        def start(broker, configured=True):
            with socket.socket() as listener:
                listener.bind(('127.0.0.1', 0))
                port = listener.getsockname()[1]
            env = {k:v for k,v in os.environ.items() if not k.startswith('P1_') and k not in ('ORCA_APPDIR', 'SCAD_LIVE_URL')}
            env.update(PORT=str(port), PLATES_DIR=str(tmp / 'plates'), LOG_LEVEL='trace')
            if configured:
                env.update(P1_IP='127.0.0.1', P1_SERIAL=SERIAL, P1_ACCESS_CODE=SECRET,
                           P1_TLS_CERT=str(tmp / 'trusted.pem'), P1_MQTT_PORT=str(broker.port))
            log = (output / ('server.log' if configured else 'unconfigured.log')).open('a')
            process = subprocess.Popen([str(binary)], cwd=tmp, env=env, stdout=log, stderr=log)
            def status():
                with urllib.request.urlopen(f'http://127.0.0.1:{port}/api/printer/status', timeout=1) as response:
                    data = response.read()
                    assert SECRET.encode() not in data
                    return json.loads(data)
            try:
                until(lambda: status())
            except Exception:
                process.terminate()
                process.wait(timeout=5)
                log.close()
                raise
            return process, log, status

        def stop(process, log):
            process.terminate()
            process.wait(timeout=5)
            log.close()

        broker = Broker(tmp / 'trusted.pem', tmp / 'trusted.key')
        process, log, status = start(broker)
        try:
            until(lambda: len(broker.requests) == 1)
            assert status()['connection'] == 'synchronizing' and not status()['ready_to_print']
            broker.send(full, retained=True)
            broker.send({'print':{'command':'push_status','msg':1,'gcode_state':'IDLE','print_error':0}})
            time.sleep(.3)
            assert not status()['synchronized']
            broker.send(full)
            until(lambda: status()['ready_to_print'])
            broker.send({'print':{'command':'push_status','msg':1,'gcode_state':'RUNNING','mc_percent':25,
                                 'ams':{'ams':[{'id':'0','tray':[{'id':'0','remain':60}]}]}}})
            until(lambda: status()['print']['percent'] == 25)
            observed = status()
            assert not observed['ready_to_print']
            assert observed['ams']['units'][0]['trays'][0]['material'] == 'PLA'
            assert observed['ams']['units'][0]['trays'][0]['remaining_percent'] == 60
            broker.send(full, topic='device/OTHER/report')
            time.sleep(.2)
            assert status()['print']['state'] == 'RUNNING'
            broker.actions.put('disconnect')
            until(lambda: status()['connection'] == 'disconnected')
            until(lambda: len(broker.requests) == 2)
            assert not status()['synchronized'] and status()['print']['state'] is None
            broker.send({'print':{'command':'push_status','msg':1,'mc_percent':50}})
            time.sleep(.2)
            assert not status()['ready_to_print']
            resumed = copy.deepcopy(full)
            resumed['print'].update(gcode_state='RUNNING', mc_percent=50)
            broker.send(resumed)
            until(lambda: status()['synchronized'])
            assert status()['print']['state'] == 'RUNNING' and not status()['ready_to_print']
            broker.send(b'broken')
            until(lambda: not status()['synchronized'])
            broker.send(full)
            until(lambda: status()['ready_to_print'])
            broker.actions.put('invalid-login-packet')
            until(lambda: status()['connection'] == 'disconnected')
        finally:
            stop(process, log)
            broker.close()
        wrong = Broker(tmp / 'other.pem', tmp / 'other.key')
        process, log, status = start(wrong)
        try:
            until(lambda: wrong.tls_failures > 0)
            until(lambda: status()['connection'] == 'disconnected')
            assert not status()['ready_to_print'] and not wrong.requests
        finally:
            stop(process, log)
            wrong.close()
        process, log, status = start(None, configured=False)
        try:
            assert status()['connection'] == 'unconfigured' and not status()['ready_to_print']
        finally:
            stop(process, log)
        for path in list(output.glob('*.log')) + list((tmp / 'plates').rglob('*')):
            if path.is_file():
                assert SECRET.encode() not in path.read_bytes(), 'credential appeared in logs/storage'
    result = dict(tls_pin_matched=True, wrong_certificate_rejected=True, mqtt_credentials=True,
                  subscription_and_pushall=True, retained_report_ignored=True, partial_before_sync_ignored=True,
                  print_ams_merge=True, disconnect_and_resync=True, unrelated_topic_ignored=True,
                  invalid_report_unsynchronized=True, secret_absent_from_api_logs_storage=True, unconfigured_unknown=True)
    (output / 'result.json').write_text(json.dumps(result, indent=2))
    print(json.dumps(result))


if __name__ == '__main__':
    main()
