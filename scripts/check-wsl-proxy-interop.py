"""WSL half of the Windows pinned-core integration test; uses only temp files."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

source=Path(__file__).resolve().parents[1]/'src-tauri/src/app_proxy/wsl/agent.py'
spec=importlib.util.spec_from_file_location('agent',source)
agent=importlib.util.module_from_spec(spec);spec.loader.exec_module(agent)
state=json.loads(sys.stdin.read())
expect_blocked=state.pop('expectBlocked',False)
state['windowsExe']=subprocess.check_output(['wslpath','-u',state['windowsExe']],text=True).strip()
with tempfile.TemporaryDirectory(prefix='nc-interop-check-') as tmp:
    home=Path(tmp)
    agent.install(home,state['windowsExe'],source.read_text())
    root=agent.root_for(home)
    (root/'state.json').write_text(json.dumps(state))
    for app in ('codex','claude'):
        binary=home/app
        binary.write_text('''#!/usr/bin/env python3
import base64, os, socket, urllib.parse
proxy=urllib.parse.urlsplit(os.environ['HTTPS_PROXY'])
auth=base64.b64encode((proxy.username+':'+proxy.password).encode()).decode()
worked=False
try:
 with socket.create_connection((proxy.hostname,proxy.port),timeout=8) as s:
  s.sendall(('CONNECT 198.18.0.10:43210 HTTP/1.1\\r\\nHost: 198.18.0.10:43210\\r\\nProxy-Authorization: Basic '+auth+'\\r\\n\\r\\n').encode())
  response=b''
  while not response.endswith(b'\\r\\n\\r\\n'):
   piece=s.recv(1)
   if not piece:raise OSError('bridge closed')
   response+=piece
  assert b'200' in response,response
  s.sendall(b'nodecloak-probe')
  assert s.recv(64)==b'via-proxy'
  worked=True
except OSError:pass
assert worked==(os.environ['EXPECT_BLOCKED']=='0'), 'wrong proxy result'
for family,address in [(socket.AF_INET,('198.18.0.10',43210)),(socket.AF_INET6,('2001:db8::1',443))]:
 try:
  with socket.socket(family) as s:s.settimeout(1);s.connect(address)
 except OSError:continue
 raise AssertionError('direct connection escaped namespace')
print('assigned node and isolated direct traffic verified')
''')
        binary.chmod(0o700)
        env=dict(os.environ,PATH=str(home)+os.pathsep+os.environ['PATH'],EXPECT_BLOCKED='1' if expect_blocked else '0')
        subprocess.run([sys.executable,str(source),'--host',str(root),app],check=True,env=env,timeout=20)
