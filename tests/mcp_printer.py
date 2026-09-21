"""Run the real Rust MCP client against disposable HTTP/SCAD/MQTT/FTPS peers."""
import json
import os
import subprocess
import sys
from print_fixture import Rig, REPO

rig=Rig(sys.argv[1],sys.argv[2])
try:
    rig.launch(); rig.seed()
    subprocess.run(['cargo','test','--locked','--test','mcp','shared_materials_ams_and_admission','--','--ignored','--nocapture'],
        cwd=REPO,env={**os.environ,'MCP_FIXTURE_URL':rig.base},check=True)
    assert not rig.broker.prints and not rig.ftp.uploads
    result=dict(actual_mcp_client=True,rest_consistency=True,common_temperature=218,quantity=10,
                stale_version_revision_refused=True,nullable_conditions=True,print_commands=0,uploads=0)
    (rig.output/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result))
finally:
    rig.close()
