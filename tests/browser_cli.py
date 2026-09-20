"""Browser + real API/OrcaSlicer: python3 tests/browser_cli.py APPDIR OUTPUT_DIR."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

REPO = Path(__file__).resolve().parents[1]


def main():
    appdir = Path(sys.argv[1]).resolve()
    output = Path(sys.argv[2]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='orca-browser-') as tmp:
        tmp = Path(tmp)
        cube = (REPO / 'tests/fixtures/cube.stl').read_bytes()
        names = ['部品/box.stl', '部品/holder.stl']
        for name in names:
            file = tmp / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(cube)

        class Scad(BaseHTTPRequestHandler):
            def do_GET(self):
                path = urllib.parse.unquote(self.path)
                if path == '/api/models':
                    data = json.dumps(names).encode()
                elif path.startswith('/models/') and path[8:] in names:
                    data = (tmp / path[8:]).read_bytes()
                else:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *_):
                pass

        source = ThreadingHTTPServer(('127.0.0.1', 0), Scad)
        threading.Thread(target=source.serve_forever, daemon=True).start()
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        base = f'http://127.0.0.1:{port}'
        env = dict(os.environ, PORT=str(port), PLATES_DIR=str(tmp / 'plates'),
                   SCAD_LIVE_URL=f'http://127.0.0.1:{source.server_port}',
                   ORCA_APPDIR=str(appdir), ORCA_TIMEOUT_SECS='60',
                   E2E_BASE_URL=base, E2E_EVIDENCE_DIR=str(output),
                   E2E_MODEL_FILE=str(tmp / names[0]))
        env.pop('DISPLAY', None)
        env.pop('WAYLAND_DISPLAY', None)
        with (output / 'server.log').open('w') as log:
            server = subprocess.Popen([str(REPO / 'target/debug/orca-server')], cwd=tmp, env=env, stdout=log, stderr=log)
            try:
                for _ in range(100):
                    if server.poll() is not None:
                        raise RuntimeError('Server exited; see server.log')
                    try:
                        with urllib.request.urlopen(base + '/healthz', timeout=1) as response:
                            if response.status == 200:
                                break
                    except OSError:
                        pass
                    time.sleep(.1)
                else:
                    raise RuntimeError('Server did not start')
                subprocess.run(['npm', 'run', 'test:e2e'], cwd=REPO / 'client', env=env, check=True)
            finally:
                server.terminate()
                server.wait(timeout=5)
                source.shutdown()
                source.server_close()


if __name__ == '__main__':
    main()
