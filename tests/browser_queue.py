"""Chromium + real API + isolated P1: python3 tests/browser_queue.py BINARY OUTPUT_DIR."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
import subprocess
import sys
import threading
from print_fixture import Rig, REPO


def run(binary, output, appdir=None):
    rig = Rig(binary, output, appdir=appdir)
    class Control(BaseHTTPRequestHandler):
        def do_GET(self): self.reply()
        def do_POST(self):
            value=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            if value.get('fail_upload'): rig.ftp.actions.put('fail')
            elif value.get('state'): rig.report(value['state'],value.get('index',-1))
            else: rig.broker.send(rig.full)
            self.reply()
        def reply(self):
            data=json.dumps(dict(prints=rig.broker.prints,uploads=rig.ftp.uploads)).encode()
            self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
        def log_message(self,*_): pass
    control=ThreadingHTTPServer(('127.0.0.1',0),Control)
    threading.Thread(target=control.serve_forever,daemon=True).start()
    try:
        rig.launch();rig.seed()
        # These are reusable compositions; the browser creates the first plate itself.
        for name in ['B · 小物ケース','C · 取り付けパーツ','D · 予備のパーツ']:
            rig.api('/api/plates/import',dict(name=name,models=[dict(name='parts/cube.stl',source='parts/cube.stl',quantity=2)]),expected=201)
        env=dict(os.environ,E2E_BASE_URL=rig.base,E2E_EVIDENCE_DIR=str(rig.output),E2E_PRINTER_CONTROL=f'http://127.0.0.1:{control.server_port}')
        subprocess.run(['npm','run','test:e2e','--','queue-live.spec.ts','--workers=1'],cwd=REPO/'client',env=env,check=True)
        assert len(rig.ftp.contents)==len(rig.broker.prints)+1  # One deliberately failed transfer.
        (rig.output/'protocol-result.json').write_text(json.dumps(dict(prints=len(rig.broker.prints),uploads=len(rig.ftp.uploads),official_cli=bool(appdir),secrets_redacted=True),indent=2))
    finally:
        control.shutdown();control.server_close();rig.close()

if __name__=='__main__': run(sys.argv[1],sys.argv[2])
