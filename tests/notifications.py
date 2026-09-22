"""Completion delivery with isolated TLS webhook, MQTT, FTPS and persistent SQLite.

python3 tests/notifications.py OUTPUT_DIR (builds the ignored Rust fixture service).
"""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import sqlite3
import ssl
import subprocess
import sys
import threading
import time
import urllib.request
from print_fixture import Rig, REPO
from printer_mqtt import until

TOKEN = 'synthetic-notification-token'


class NotificationRig(Rig):
    def launch(self):
        requests = len(self.broker.requests)
        self.process = subprocess.Popen([self.binary, '--exact', 'notifications::tests::fixture_service', '--ignored', '--nocapture'],
            cwd=self.root, env=self.env, stdout=self.log, stderr=self.log)
        until(lambda: urllib.request.urlopen(self.base+'/healthz', timeout=1).status == 200, 30)
        until(lambda: len(self.broker.requests) > requests)

    def rows(self):
        with sqlite3.connect(self.store/'orca.sqlite3') as db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute('SELECT * FROM print_notifications ORDER BY rowid')]

    def sql(self, command):
        with sqlite3.connect(self.store/'orca.sqlite3') as db: db.executescript(command)

    def start_job(self):
        self.idle(); job = self.add(); count = len(self.broker.prints)
        self.next(job); until(lambda: len(self.broker.prints) == count+1)
        return job

    def finish(self):
        self.report('RUNNING'); self.phase('printing')
        self.report('FINISH'); self.phase('awaiting_removal')

    def remove_current(self):
        self.send(dict(type='discard', expected_job=self.api()['current']['id'], cleared=True))
        assert self.api()['current'] is None


def run(output):
    output = Path(output).resolve(); output.mkdir(parents=True, exist_ok=True)
    built = subprocess.check_output(['cargo', 'test', '--lib', '--no-run', '--message-format=json'], cwd=REPO, text=True)
    binary = next(v['executable'] for line in built.splitlines() if (v := json.loads(line)).get('reason') == 'compiler-artifact' and v.get('executable') and v['target']['kind'] == ['lib'])
    rig = NotificationRig(binary, output); actions = queue.Queue(); requests = []; results = {}
    class Webhook(BaseHTTPRequestHandler):
        def do_POST(self):
            payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            requests.append(dict(time=time.monotonic(), path=self.path, payload=payload))
            assert self.path == f'/api/webhooks/123/{TOKEN}?wait=true'
            action = actions.get_nowait() if not actions.empty() else (200, {'id': str(1000+len(requests))}, 0)
            status, body, delay = action
            time.sleep(delay)
            data = json.dumps(body).encode()
            try:
                self.send_response(status); self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)
            except (BrokenPipeError, ConnectionResetError, ssl.SSLError): pass
        def log_message(self, *_): pass
    webhook = ThreadingHTTPServer(('127.0.0.1', 0), Webhook)
    subprocess.run(['openssl', 'req', '-config', '/dev/null', '-x509', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:P-256',
        '-addext', 'subjectAltName=DNS:localhost', '-nodes', '-days', '1', '-subj', '/CN=localhost',
        '-keyout', str(rig.root/'webhook.key'), '-out', str(rig.root/'webhook.pem')], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); tls.load_cert_chain(rig.root/'webhook.pem', rig.root/'webhook.key')
    webhook.socket = tls.wrap_socket(webhook.socket, server_side=True)
    threading.Thread(target=webhook.serve_forever, daemon=True).start()
    rig.env.pop('DISCORD_WEBHOOK_URL', None); rig.env.pop('ORCA_PUBLIC_URL', None)
    rig.env.update(NOTIFICATION_TEST_ENABLED='1', NOTIFICATION_TEST_CERT=str(rig.root/'webhook.pem'),
        NOTIFICATION_TEST_URL=f'https://localhost:{webhook.server_port}/api/webhooks/123/{TOKEN}?wait=true')
    try:
        rig.launch(); rig.seed()
        plate = rig.api('/api/plates/'+rig.plate['id']); plate['name'] = '@everyone **fixture** <@123>'
        rig.plate = rig.api('/api/plates/'+plate.pop('id'), plate, 'PUT')
        first = rig.start_job()
        command = rig.broker.prints[-1]
        rig.broker.send({'print': dict(command='project_file', sequence_id=command['sequence_id'], result='success')})
        until(lambda: rig.api()['printer']['start']['phase'] == 'accepted')
        value = copy.deepcopy(rig.full); value['print'].update(gcode_state='RUNNING', mc_percent=100, subtask_name=command['subtask_name'], gcode_file=command['file'])
        rig.broker.send(value); rig.phase('printing'); time.sleep(1.1)
        assert not requests and not rig.rows()
        rig.report('FINISH'); rig.phase('awaiting_removal')
        until(lambda: rig.rows() and rig.rows()[0]['state'] == 'sent')
        sent = rig.rows()[0]; assert sent['job_id'] == first['id'] and sent['message_id'] == '1001'
        text = requests[0]['payload']['content']
        assert '取り外し待ち' in text and first['id'] in text and 'https://orca.example/queue?printer_id=p1' in text
        assert '@everyone' not in text and '<@123>' not in text and requests[0]['payload']['allowed_mentions'] == {'parse': []}
        for _ in range(3): rig.report('FINISH')
        rig.stop(); rig.launch(); rig.report('FINISH'); rig.phase('awaiting_removal'); time.sleep(1.2)
        assert len(requests) == 1 and rig.rows()[0]['tries'] == 1
        rig.remove_current(); assert len(rig.rows()) == 1
        results['finish_only_one_notification_across_reports_restart_and_removal'] = True
        # A pending outcome survives immediate removal. Slow webhook does not hold queue/MQTT.
        actions.put((200, {'id':'2000'}, 1.2)); second = rig.start_job(); rig.finish()
        until(lambda: len(requests) == 2)
        started = time.monotonic(); rig.remove_current(); assert time.monotonic()-started < .5
        until(lambda: rig.rows()[-1]['state'] == 'unknown')
        assert rig.rows()[-1]['job_id'] == second['id']
        until(lambda: rig.rows()[-1]['state'] == 'sent')
        assert rig.rows()[-1]['tries'] == 2  # Reply loss can duplicate an accepted message.
        results['timeout_does_not_block_removal_and_retains_uncertainty'] = True
        # A rate limit persists over restart; no request before the requested delay.
        actions.put((429, {'retry_after':3.0}, 0)); rig.start_job(); rig.finish()
        until(lambda: rig.rows()[-1]['result'] == '429'); limited = requests[-1]['time']; count = len(requests)
        rig.stop(); rig.launch(); rig.report('FINISH'); time.sleep(.5); assert len(requests) == count
        until(lambda: rig.rows()[-1]['state'] == 'sent')
        assert requests[-1]['time']-limited >= 3 and rig.rows()[-1]['tries'] == 2
        rig.remove_current(); results['rate_limit_survives_restart'] = True
        for label, responses, state, tries in [
            ('server_error_recovers', [(503, {}, 0), (200, {'id':'3000'}, 0)], 'sent', 2),
            ('server_error_stops', [(503, {}, 0)]*3, 'failed', 3),
            ('permanent_error_stops', [(401, {'message':TOKEN}, 0)], 'failed', 1),
            ('lost_response_stops_uncertain', [(200, {'id':'4000'}, 1.2)]*3, 'unknown', 3),
        ]:
            for action in responses: actions.put(action)
            rig.start_job(); rig.finish()
            until(lambda: rig.rows()[-1]['state'] == state and rig.rows()[-1]['tries'] == tries, 20)
            count = len(requests); time.sleep(1.2); assert len(requests) == count
            assert rig.api()['current']['state'] == 'awaiting_removal'
            rig.remove_current(); results[label] = True
        # Process loss after the HTTP request is uncertain; bounded replay resumes from disk.
        actions.put((200, {'id':'5000'}, 1.2)); rig.start_job(); count = len(requests); rig.finish()
        until(lambda: len(requests) > count)
        assert rig.rows()[-1]['state'] == 'sending'
        rig.stop(kill=True); rig.launch(); rig.report('FINISH')
        assert rig.rows()[-1]['state'] == 'unknown'
        until(lambda: rig.rows()[-1]['state'] == 'sent')
        assert rig.rows()[-1]['tries'] == 2; rig.remove_current()
        results['kill_in_flight_recovers_with_possible_duplicate'] = True
        # Save failure after an acknowledgement retries storage, without another HTTP delivery.
        rig.sql("CREATE TRIGGER fixture_fail_ack BEFORE UPDATE ON print_notifications WHEN NEW.state='sent' BEGIN SELECT RAISE(ABORT,'fixture'); END;")
        rig.start_job(); count = len(requests); rig.finish(); until(lambda: len(requests) > count)
        time.sleep(.2); assert rig.rows()[-1]['state'] == 'sending'
        rig.sql('DROP TRIGGER fixture_fail_ack;'); count = len(requests)
        until(lambda: rig.rows()[-1]['state'] == 'sent'); assert len(requests) == count
        rig.remove_current(); results['ack_save_retry_does_not_resend'] = True
        # A rejected attempt and cancellation generate no completion; explicit retry can complete.
        count = len(requests); job = rig.start_job(); command = rig.broker.prints[-1]
        rig.broker.send({'print':dict(command='project_file', sequence_id=command['sequence_id'], result='fail')})
        rig.phase('needs_attention'); time.sleep(1.2); assert len(requests) == count
        rig.idle(); rig.send(dict(type='retry', expected_job=job['id'], cleared=True)); old = command
        until(lambda: rig.broker.prints[-1] != old); rig.finish()
        until(lambda: rig.rows()[-1]['state'] == 'sent'); assert len(requests) == count+1
        rig.remove_current(); cancelled = rig.add(); rig.send(dict(type='remove',job_id=cancelled['id']))
        time.sleep(1.2); assert len(requests) == count+1
        results['failed_and_cancelled_do_not_notify_retry_attempt_does'] = True
        # Disable then re-enable across a schema-7 migration: existing FINISH is not backfilled.
        rig.stop(); rig.env['NOTIFICATION_TEST_ENABLED'] = '0'; rig.launch()
        rig.start_job(); rig.finish(); count = len(requests); before = len(rig.rows()); time.sleep(1.2)
        assert len(requests) == count and len(rig.rows()) == before
        rig.stop(); rig.sql('ALTER TABLE plates DROP COLUMN brim_enabled; ALTER TABLE plates DROP COLUMN deleted; ALTER TABLE plates DROP COLUMN sparse_infill_pattern; ALTER TABLE plates DROP COLUMN sparse_infill_density; ALTER TABLE plates DROP COLUMN wall_loops; ALTER TABLE default_settings DROP COLUMN sparse_infill_pattern; ALTER TABLE default_settings DROP COLUMN sparse_infill_density; ALTER TABLE default_settings DROP COLUMN wall_loops; DROP TABLE print_notifications; ALTER TABLE print_jobs DROP COLUMN estimate_json; PRAGMA user_version=7;')
        rig.env['NOTIFICATION_TEST_ENABLED'] = '1'; rig.launch(); rig.report('FINISH'); rig.phase('awaiting_removal'); time.sleep(1.2)
        assert not rig.rows() and len(requests) == count
        rig.remove_current(); results['unset_disables_and_schema_upgrade_does_not_backfill'] = True
        for path in ['/api/queue?printer_id=p1','/api/printers','/api/plates','/api/about']:
            assert TOKEN not in json.dumps(rig.api(path))
        assert TOKEN not in (output/'server.log').read_text()
        results.update(secrets_redacted=True, https_verified=True, requests=len(requests), print_attempts=len(rig.broker.prints))
        (output/'result.json').write_text(json.dumps(results, indent=2)); print(json.dumps(results))
    finally:
        webhook.shutdown(); webhook.server_close(); rig.close()


if __name__ == '__main__': run(sys.argv[1])
