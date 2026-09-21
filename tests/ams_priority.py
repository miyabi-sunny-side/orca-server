"""Real HTTP/SQLite + isolated MQTT/FTPS: priority, refill control, same running job."""
import copy
import json
import os
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import sys
from print_fixture import Rig, until


def main():
    rig=Rig(sys.argv[1],sys.argv[2]);options=[]
    original=rig.broker.on_request
    def on_request(value):
        if value.get('print',{}).get('command')=='print_option':
            assert set(value['print'])=={'command','sequence_id','auto_switch_filament'}
            assert isinstance(value['print']['auto_switch_filament'],bool)
            options.append(value)
        else:original(value)
    rig.broker.on_request=on_request
    try:
        rig.launch();rig.seed();fid=rig.materials[0]['id'];root='/api/printers/p1/ams'
        def inv():return rig.api(root)
        def resolve():return rig.api(root+'/resolve?filament_id='+fid)
        assert resolve()['preferred_slot']['slot_index']==0
        rig.api(root+'/auto-refill',dict(enabled=True),'PUT',409);assert not options
        # An explicit second load of the same product/color shares all material settings.
        report=copy.deepcopy(rig.full);report['print']['ams']['tray_exist_bits']='b'
        report['print']['ams']['ams'][0]['tray'][1].update(tray_type='PLA',tray_color='FFFFFFFF')
        report['print'].update(support_filament_backup=True,home_flag=0,filam_bak=[3])
        rig.broker.send(report);until(lambda:inv()['slots'][1]['reported']['present'])
        slot=inv()['slots'][1];rig.api(root+'/'+slot['id'],dict(revision=slot['revision'],filament_id=fid),'PUT',204)
        group=resolve()['candidates'];assert [s['slot_index'] for s in group]==[0,1]
        priority=dict(filament_id=fid,order=[dict(id=s['id'],revision=s['revision']) for s in reversed(group)])
        rig.api(root+'/priority',priority,'PUT',204);assert resolve()['preferred_slot']['slot_index']==1
        rig.api(root+'/priority',priority,'PUT',409)
        assert inv()['auto_refill']['enabled'] is False and inv()['auto_refill']['supported'] is True
        # Publish only requests the setting: success must not be invented before a new report.
        rig.api(root+'/auto-refill',dict(enabled=True),'PUT',202);until(lambda:len(options)==1)
        assert inv()['auto_refill']['enabled'] is False
        rig.broker.send(dict(print=dict(command='push_status',msg=1,home_flag=1024)))
        until(lambda:inv()['auto_refill']['enabled'] is True)
        assert inv()['slots'][0]['backup_peers']==[1]
        rig.broker.send(dict(print=dict(command='push_status',msg=1,support_filament_backup=False)))
        until(lambda:inv()['auto_refill']['supported'] is False)
        rig.api(root+'/auto-refill',dict(enabled=True),'PUT',409);assert len(options)==1
        if os.environ.get('AMS_BROWSER'):
            class Control(BaseHTTPRequestHandler):
                def do_POST(self):
                    rig.broker.send(json.loads(self.rfile.read(int(self.headers['Content-Length']))));self.send_response(204);self.end_headers()
                def log_message(self,*_):pass
            control=ThreadingHTTPServer(('127.0.0.1',0),Control);threading.Thread(target=control.serve_forever,daemon=True).start()
            try:
                env=dict(os.environ,E2E_BASE_URL=rig.base,E2E_EVIDENCE_DIR=str(rig.output/'browser'),E2E_AMS_CONTEXT=json.dumps(dict(resolve=root+'/resolve?filament_id='+fid,control=f'http://127.0.0.1:{control.server_port}')))
                subprocess.run(['npm','run','test:e2e','--','--workers=1'],cwd=Path(__file__).resolve().parents[1]/'client',env=env,check=True)
            finally:control.shutdown();control.server_close()
        # Backup changes the physical tray, never the job identity, artifact or print send count.
        rig.full=report
        preferred=resolve()['preferred_slot'];job=rig.add(0)
        assert job['ams_slot_id']==preferred['id']
        rig.next(job);until(lambda:len(rig.broker.prints)==1)
        assert rig.broker.prints[0]['ams_mapping']==[preferred['slot_index']]
        rig.full['print']['ams']['tray_now']=str(preferred['slot_index'])
        rig.report("RUNNING");rig.phase("printing")
        before=rig.api()['current'];switched=1 if preferred['slot_index']==0 else 0
        rig.broker.send(dict(print=dict(command='push_status',msg=1,ams=dict(tray_now=str(switched)),mc_percent=20)))
        until(lambda:rig.api()['current']['actual_ams_slot']==switched)
        after=rig.api()['current'];assert after['ams_slot_id']==before['ams_slot_id']
        assert after['id']==before['id'] and after['attempt_id']==before['attempt_id'] and after['state']=='printing'
        assert len(rig.broker.prints)==1 and len(rig.ftp.contents)==1
        sent=len(options)
        rig.stop();rig.launch();assert not inv()['current']
        rig.api(root+'/resolve?filament_id='+fid,expected=409)
        rig.api(root+'/auto-refill',dict(enabled=False),'PUT',409)
        assert len(options)==sent
        (rig.output/'verified.json').write_text(json.dumps(dict(priority=True,stale_rejected=True,setting_confirmed_by_report=True,native_switch_same_job=True,no_reprint=True,disconnected_refused=True),indent=2))
    finally:rig.close()
    print('AMS priority, native refill setting and no-reprint contract passed')

if __name__=='__main__':main()
