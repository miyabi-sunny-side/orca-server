"""Observe real FTPS/TLS and MQTT for a saved Orca 2.4.2 cube print.

python3 tests/printer_start.py BINARY OUTPUT_DIR
Only disposable loopback peers are used; no hardware print is started.
"""
import copy
import json
from pathlib import Path
import queue
import socket
import ssl
import sys
import threading
import time
from printer_mqtt import Broker, SERIAL, SECRET, until, certificate

REPO = Path(__file__).resolve().parents[1]
ARTIFACT = (REPO / 'tests/fixtures/p1_print.gcode.3mf').read_bytes()


class PrintBroker(Broker):
    def __init__(self, cert, key, serial=SERIAL):
        self.prints = []
        super().__init__(cert, key, serial)

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
        self.contents = []
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
                        self.contents.append(content)
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
    from print_fixture import Rig
    version=sys.argv[3] if len(sys.argv)>3 else 'v3'
    rig=Rig(sys.argv[1],sys.argv[2],version);rig.env['P1_START_TIMEOUT_SECS']='3'
    certificate(rig.root,'other',version);wrong=Ftps(rig.root/'other.pem',rig.root/'other.key')
    def phase(name):return until(lambda:rig.api()['printer']['start'] and rig.api()['printer']['start']['phase']==name)
    def retry(job):return rig.send(dict(type='retry',expected_job=job['id'],cleared=True))
    try:
        rig.launch();rig.seed();job=rig.add();rig.next(job);until(lambda:len(rig.broker.prints)==1)
        command=rig.broker.prints[0]
        assert command['url']=='ftp:///'+rig.ftp.uploads[0]
        assert command['param']=='Metadata/plate_1.gcode' and command['ams_mapping']==[3] and command['use_ams']
        current=rig.api()['current'];assert rig.ftp.contents[-1]==(rig.store/current['artifact_path']/'print.gcode.3mf').read_bytes()
        phase('awaiting_confirmation')
        rig.broker.send({'print':dict(command='project_file',sequence_id='wrong',result='success')})
        time.sleep(.1);phase('awaiting_confirmation')
        rig.broker.send({'print':dict(command='project_file',sequence_id=command['sequence_id'],result='success')})
        phase('accepted');assert rig.api()['current']['state']=='preparing'
        unrelated=copy.deepcopy(rig.full);unrelated['print'].update(gcode_state='RUNNING',subtask_name='not-our-job',gcode_file='other.gcode.3mf')
        rig.broker.send(unrelated);time.sleep(.1);phase('accepted')
        rig.report('RUNNING');phase('printing');rig.broker.actions.put('disconnect');phase('unknown')
        until(lambda:len(rig.broker.requests)==2);assert len(rig.broker.prints)==1
        rig.report('RUNNING');phase('printing');rig.report('FINISH');phase('finished')
        rig.send(dict(type='discard',expected_job=job['id'],cleared=True))
        rig.idle();job=rig.add();rig.ftp.actions.put('fail');rig.next(job);phase('upload_failed');assert len(rig.broker.prints)==1
        rig.ftp.actions.put('wait');retry(job);assert rig.ftp.received.wait(10)
        busy=copy.deepcopy(rig.full);busy['print']['gcode_state']='RUNNING';rig.broker.send(busy)
        until(lambda:not rig.api()['printer']['ready_to_print']);rig.ftp.gate.set();phase('not_sent');assert len(rig.broker.prints)==1
        rig.idle();retry(job);until(lambda:len(rig.broker.prints)==2)
        rejected=rig.broker.prints[-1]
        rig.broker.send({'print':dict(command='project_file',sequence_id=rejected['sequence_id'],result='fail')});phase('rejected')
        retry(job);until(lambda:len(rig.broker.prints)==3);phase('unknown')
        until(lambda:len(rig.broker.requests)==3);assert len(rig.broker.prints)==3
        rig.idle();rig.send(dict(type='retry',expected_job=job['id'],cleared=False),409)
        rig.send(dict(type='discard',expected_job=job['id'],cleared=True))
        settings={k:v for k,v in rig.api('/api/printers/p1').items() if k not in ('id','status','machine','configuration_error')}
        settings['ftps_port']=wrong.port;requests=len(rig.broker.requests)
        rig.api('/api/printers/p1',settings,'PUT');until(lambda:len(rig.broker.requests)>requests);rig.idle()
        job=rig.add();rig.next(job);phase('upload_failed')
        assert wrong.tls_failures and not wrong.uploads and len(rig.broker.prints)==3
    finally:
        rig.close();wrong.close()
    result=dict(certificate_version=version,execution_bytes_uploaded=True,implicit_tls_session_reused=True,passive_host_ignored=True,
        selected_ams_mapping=True,upload_failure_no_command=True,state_change_during_upload_no_command=True,
        ack_distinct_from_printing=True,matching_status_required=True,timeout_and_reconnect_no_replay=True,
        explicit_recovery_required=True,wrong_ftp_certificate_refused=True,secrets_redacted=True)
    (Path(sys.argv[2])/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))


if __name__=='__main__':main()
