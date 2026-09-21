"""Real MCP continuation through the shared queue and isolated printer protocols."""
import json
import os
import subprocess
import sys
from print_fixture import Rig, REPO, printer_control, until

rig=Rig(sys.argv[1],sys.argv[2]);control=printer_control(rig)
try:
    rig.launch();rig.seed();rig.add(3);rig.add(0)
    # Compare full MCP/REST snapshots only after their asynchronous estimates settle.
    until(lambda: all(job["estimate"]["state"] == "ready" for job in rig.api()["waiting"]))
    subprocess.run(['cargo','test','--locked','--test','mcp','queue_continuation','--','--ignored','--nocapture'],
        cwd=REPO,env={**os.environ,'MCP_FIXTURE_URL':rig.base,'MCP_PRINTER_CONTROL':f'http://127.0.0.1:{control.server_port}'},check=True)
    assert len(rig.broker.prints)==len(rig.ftp.uploads)==2
    assert rig.api()['current'] is None and not rig.api()['waiting']
    result=dict(real_mcp=True,prints=2,uploads=2,finish_without_start=True,exact_replay=True,
                stale_target_refused=True,disconnected_refused=True,held_head_not_skipped=True)
    (rig.output/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
finally:
    control.shutdown();control.server_close();rig.close()
