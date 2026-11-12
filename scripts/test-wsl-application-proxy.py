"""Real isolated Linux networking tests; run inside WSL, never alters host routes."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import socket
import socketserver
import threading
import pty
import fcntl
import termios
import time
import select

SOURCE = Path(__file__).resolve().parents[1] / "src-tauri/src/app_proxy/wsl/agent.py"
spec = importlib.util.spec_from_file_location("agent", SOURCE)
agent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent)


class Integration(unittest.TestCase):
    def test_terminal_ctrl_c_reaches_tool_once_without_killing_the_proxy_worker(self):
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp);root=agent.root_for(home)
            agent.install(home,'/missing-relay',SOURCE.read_text())
            (root/'state.json').write_text(json.dumps({'enabled':True,'windowsExe':'/missing-relay','apps':{'codex':{'port':23456,'password':'a'*64}}}))
            binary=home/'codex'
            binary.write_text('''#!/usr/bin/env python3
import signal,time,os,socket,urllib.parse
count=0
def interrupted(sig,frame):
 global count
 count+=1
signal.signal(signal.SIGINT,interrupted)
print('READY',flush=True)
deadline=time.monotonic()+5
while not count and time.monotonic()<deadline:time.sleep(.02)
time.sleep(.2)
p=urllib.parse.urlsplit(os.environ['HTTP_PROXY'])
s=socket.create_connection((p.hostname,p.port),timeout=2)
print('COUNT='+str(count),flush=True)
''');binary.chmod(0o700)
            master,slave=pty.openpty()
            def controlling_terminal():
                os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
            process=subprocess.Popen([sys.executable,str(SOURCE),'--host',str(root),'codex'],stdin=slave,stdout=slave,stderr=slave,env=dict(os.environ,PATH=str(home)+':'+os.environ['PATH']),preexec_fn=controlling_terminal,close_fds=True)
            os.close(slave);output=b''
            try:
                deadline=time.monotonic()+8
                while b'READY' not in output and time.monotonic()<deadline:
                    if select.select([master],[],[],.1)[0]:output+=os.read(master,4096)
                self.assertIn(b'READY',output);os.write(master,b'\x03')
                while process.poll() is None and time.monotonic()<deadline:
                    if select.select([master],[],[],.1)[0]:
                        try:output+=os.read(master,4096)
                        except OSError:break
                self.assertEqual(process.wait(timeout=3),0,output.decode(errors='replace'))
                self.assertIn(b'COUNT=1',output)
                self.assertNotIn(b'COUNT=2',output)
                self.assertNotIn(b'KeyboardInterrupt',output)
            finally:
                if process.poll() is None:process.kill();process.wait()
                os.close(master)

    def test_tool_exit_releases_a_bridge_that_never_replied(self):
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp);root=agent.root_for(home);relay=home/'relay';pidfile=home/'relay.pid'
            relay.write_text('#!/usr/bin/env python3\nimport os,time\nopen('+repr(str(pidfile))+',"w").write(str(os.getpid()))\ntime.sleep(30)\n');relay.chmod(0o700)
            agent.install(home,str(relay),SOURCE.read_text())
            (root/'state.json').write_text(json.dumps({'enabled':True,'windowsExe':str(relay),'apps':{'codex':{'port':23456,'password':'a'*64}}}))
            binary=home/'codex';binary.write_text('#!/usr/bin/env python3\nimport os,socket,time,urllib.parse\np=urllib.parse.urlsplit(os.environ["HTTP_PROXY"]);s=socket.create_connection((p.hostname,p.port));s.sendall(b"GET http://fixture.invalid/ HTTP/1.1\\r\\nHost: fixture.invalid\\r\\n\\r\\n")\nfor _ in range(100):\n if os.path.exists('+repr(str(pidfile))+'):break\n time.sleep(.02)\n');binary.chmod(0o700)
            env=dict(os.environ,PATH=str(home)+':'+os.environ['PATH'])
            result=subprocess.run([sys.executable,str(SOURCE),'--host',str(root),'codex'],capture_output=True,text=True,env=env,timeout=10)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertTrue(pidfile.exists(),'fixture bridge did not start')
            pid=int(pidfile.read_text())
            try:
                with self.assertRaises(ProcessLookupError):os.kill(pid,0)
            finally:
                try:os.kill(pid,15)
                except ProcessLookupError:pass

    @unittest.skipUnless(os.environ.get('WSL_DISTRO_NAME'),'requires WSL path conversion')
    def test_deployed_management_protocol_installs_updates_and_removes_in_a_temporary_home(self):
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp);settings=home/'.vscode-server/data/Machine/settings.json'
            settings.parent.mkdir(parents=True);settings.write_text('{ // preserve this\n "editor.fontSize":17,\n "claudeCode.claudeProcessWrapper":"/custom-wrapper"\n}')
            source=SOURCE.read_text()
            def manage(action,payload):
                return subprocess.run([sys.executable,'-c',"import sys; source=sys.stdin.read(); exec(compile(source, 'nodecloak_wsl.py', 'exec'))",'--manage',action,json.dumps(payload)],input=source,capture_output=True,text=True,env=dict(os.environ,HOME=tmp),timeout=15)
            result=manage('install',{'windowsExe':'C:\\NodeCloak\\nodecloak.exe'})
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertTrue(json.loads(result.stdout)['installed'])
            # A desktop update must refresh the installed launcher code before
            # enabling new sessions, rather than continuing an old adapter.
            root=agent.root_for(home);(root/'agent.py').write_text('raise SystemExit(44)\n')
            result=manage('state',{'enabled':True,'apps':{'claude':{'port':23456,'password':'a'*64}}})
            self.assertEqual(result.returncode,0,result.stderr)
            version_check=subprocess.run([sys.executable,str(root/'agent.py'),'--host',str(root),'claude','--version'],capture_output=True,text=True,env=dict(os.environ,PATH='/nonexistent'),timeout=10)
            self.assertEqual(version_check.returncode,1,'stale launcher ran after a desktop update')
            self.assertIn('NodeCloak',version_check.stderr)
            self.assertNotEqual(manage('remove',{}).returncode,0,'protected integration was removed without Stop')
            self.assertEqual(manage('state',{'enabled':False,'apps':{}}).returncode,0)
            result=manage('remove',{});self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(agent.jsonc(settings.read_text())['claudeCode.claudeProcessWrapper'],'/custom-wrapper')
            self.assertIn('// preserve this',settings.read_text())

    def test_protected_launcher_preserves_exit_status_and_forwards_termination(self):
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp);root=agent.root_for(home)
            agent.install(home,'/unavailable-relay',SOURCE.read_text())
            (root/'state.json').write_text(json.dumps({'enabled':True,'windowsExe':'/unavailable-relay','apps':{'codex':{'port':23456,'password':'a'*64}}}))
            binary=home/'codex';binary.write_text('#!/usr/bin/env python3\nimport sys, time\nprint("ready",flush=True)\nif len(sys.argv)>1:sys.exit(17)\ntime.sleep(30)\n');binary.chmod(0o700)
            env=dict(os.environ,PATH=str(home)+':'+os.environ['PATH'])
            invocation=[sys.executable,str(SOURCE),'--host',str(root),'codex']
            result=subprocess.run(invocation+['exit'],capture_output=True,text=True,env=env,timeout=10)
            self.assertEqual(result.returncode,17);self.assertEqual(result.stdout.strip(),'ready')
            process=subprocess.Popen(invocation,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=env)
            try:
                self.assertEqual(process.stdout.readline().strip(),'ready')
                process.terminate();self.assertEqual(process.wait(timeout=10),143)
            finally:
                if process.poll() is None:process.kill();process.wait()
                process.stdout.close();process.stderr.close()

    def test_explicit_stop_restores_network_for_new_launches_only(self):
        class Listener(socketserver.BaseRequestHandler):
            def handle(self):self.request.sendall(b'host reached')
        with tempfile.TemporaryDirectory() as tmp, socketserver.ThreadingTCPServer(('127.0.0.1',0),Listener) as server:
            threading.Thread(target=server.serve_forever,daemon=True).start()
            home=Path(tmp);root=agent.root_for(home);agent.install(home,'/unavailable',SOURCE.read_text())
            binary=home/'codex';binary.write_text('#!/usr/bin/env python3\nimport os,socket\ntry:\n s=socket.create_connection(("127.0.0.1",int(os.environ["TARGET_PORT"])),timeout=1);print(s.recv(20).decode())\nexcept OSError:print("blocked")\n');binary.chmod(0o700)
            env=dict(os.environ,PATH=str(home)+':'+os.environ['PATH'],TARGET_PORT=str(server.server_address[1]))
            state={'enabled':True,'windowsExe':'/unavailable','apps':{'codex':{'port':23456,'password':'a'*64}}}
            for enabled,expected in [(True,'blocked'),(False,'host reached')]:
                state['enabled']=enabled
                (root/'state.json').write_text(json.dumps(state))
                result=subprocess.run([sys.executable,str(SOURCE),'--host',str(root),'codex'],capture_output=True,text=True,env=env,timeout=10)
                self.assertEqual(result.returncode,0,result.stderr);self.assertEqual(result.stdout.strip(),expected)
            server.shutdown()

    def test_custom_vscode_wrapper_uses_the_original_program_not_the_cli_hook(self):
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp);root=agent.root_for(home);native=home/'native';native.mkdir();exe=native/'claude'
            exe.write_text('#!/bin/sh\nexit 0\n');exe.chmod(0o700)
            metadata={'settings':{str(home/'.vscode-server/data/Machine/settings.json'):'claude'}}
            with mock.patch.dict(os.environ,HOME=str(home),PATH=str(root/'bin')+':'+str(native)):
                command=agent.real_command(root,'claude-vscode',['/home/james/.vscode-server/extensions/test/resources/native-binary/claude','--version'],metadata)
            self.assertEqual(command[0],str(exe))

    def test_interrupted_install_retry_still_restores_the_original_wrapper(self):
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp); settings=home/'.vscode-server/data/Machine/settings.json'
            settings.parent.mkdir(parents=True);settings.write_text('{"claudeCode.claudeProcessWrapper":"/original-wrapper"}')
            original=agent.atomic
            def interrupt(file,text,mode=0o600):
                if Path(file).name=='.profile':raise OSError('simulated write failure')
                original(file,text,mode)
            with mock.patch.object(agent,'atomic',side_effect=interrupt):
                with self.assertRaises(OSError):agent.install(home,'/windows.exe',SOURCE.read_text())
            agent.install(home,'/windows.exe',SOURCE.read_text());agent.uninstall(home)
            self.assertEqual(agent.jsonc(settings.read_text())['claudeCode.claudeProcessWrapper'],'/original-wrapper')

    def test_shell_files_without_final_newline_are_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp);file=home/'.bashrc';file.write_text('export MY_SETTING=yes')
            agent.install(home,'/windows.exe',SOURCE.read_text())
            self.assertIn('export MY_SETTING=yes\n',file.read_text())
            agent.uninstall(home);self.assertEqual(file.read_text(),'export MY_SETTING=yes')

    def test_real_namespace_routes_proxy_but_blocks_direct_ipv4_ipv6_and_udp(self):
        self.namespace_case(dead=False)

    def test_dead_bridge_never_retries_over_direct_network(self):
        self.namespace_case(dead=True)

    def test_claude_retains_only_the_explicit_vscode_loopback_port(self):
        self.namespace_case(dead=False,with_ide=True)

    def namespace_case(self, dead,with_ide=False):
        # The host fixture is reachable normally; namespace children can only
        # reach it through the same Unix/stdin bridge used by Windows interop.
        hits=[]
        class Proxy(socketserver.BaseRequestHandler):
            def handle(self):
                data=self.request.recv(65536)
                hits.append(data)
                self.request.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nproxied')
        class Ide(socketserver.BaseRequestHandler):
            def handle(self):
                if self.request.recv(20)==b'editor':self.request.sendall(b'editor-ok')
        with socketserver.ThreadingTCPServer(('127.0.0.1',0),Proxy) as server, socketserver.ThreadingTCPServer(('127.0.0.1',0),Ide) as ide, tempfile.TemporaryDirectory() as tmp:
            threading.Thread(target=server.serve_forever,daemon=True).start()
            threading.Thread(target=ide.serve_forever,daemon=True).start()
            home=Path(tmp); root=agent.root_for(home)
            relay=home/'nodecloak.exe'
            relay.write_text('''#!/usr/bin/env python3
import os, socket, sys, threading
s=socket.create_connection(('127.0.0.1',int(sys.argv[2])))
def upload():
 while data:=os.read(0,65536):s.sendall(data)
 s.shutdown(socket.SHUT_WR)
threading.Thread(target=upload,daemon=True).start()
while data:=s.recv(65536):os.write(1,data)
''');relay.chmod(0o700)
            agent.install(home,str(relay),SOURCE.read_text())
            app='claude' if with_ide else 'codex'
            state={'enabled':True,'windowsExe':str(relay if not dead else home/'missing.exe'),'apps':{app:{'port':server.server_address[1],'password':'a'*64}}}
            (root/'state.json').write_text(json.dumps(state))
            binary=home/app
            binary.write_text('''#!/usr/bin/env python3
import json, os, socket, urllib.request, subprocess
result={}
caps=[line.split()[1] for line in open('/proc/self/status') if line.startswith('CapEff:')][0]
assert int(caps,16)==0,'capabilities must be dropped before tool execution'
if os.environ.get('WITH_IDE'):
 with socket.create_connection(('127.0.0.1',int(os.environ['CLAUDE_CODE_SSE_PORT'])),timeout=2) as s:
  s.sendall(b'editor');assert s.recv(20)==b'editor-ok'
if os.path.exists('/mnt/c/Windows/System32/cmd.exe'):
 result['interop']=subprocess.run(['/mnt/c/Windows/System32/cmd.exe','/c','echo','must-not-run'],capture_output=True).returncode
try:result['proxy']=urllib.request.urlopen('http://fixture.invalid/',timeout=3).read().decode()
except Exception:result['proxy']='blocked'
for key,family,kind,address in [('v4',socket.AF_INET,socket.SOCK_STREAM,('198.18.0.1',443)),('v6',socket.AF_INET6,socket.SOCK_STREAM,('2001:db8::1',443)),('udp',socket.AF_INET,socket.SOCK_DGRAM,('198.18.0.1',53)),('host',socket.AF_INET,socket.SOCK_STREAM,('127.0.0.1',int(os.environ['FIXTURE_PORT'])))]:
 try:
  with socket.socket(family,kind) as s:
   s.settimeout(1)
   if kind==socket.SOCK_DGRAM:s.sendto(b'not allowed',address)
   else:s.connect(address)
  result[key]='leaked'
 except OSError:result[key]='blocked'
print(json.dumps(result))
''');binary.chmod(0o700)
            env=dict(os.environ,PATH=str(home)+os.pathsep+os.environ['PATH'],FIXTURE_PORT=str(server.server_address[1]),HOME=str(home))
            if with_ide:env.update(CLAUDE_CODE_SSE_PORT=str(ide.server_address[1]),WITH_IDE='1')
            proc=subprocess.run([sys.executable,str(SOURCE),'--host',str(root),app],capture_output=True,text=True,env=env,timeout=20)
            self.assertEqual(proc.returncode,0,proc.stderr)
            result=json.loads(proc.stdout)
            if 'interop' in result:self.assertNotEqual(result.pop('interop'),0,'Windows interop escaped isolation')
            self.assertEqual(result,{'proxy':'blocked' if dead else 'proxied','v4':'blocked','v6':'blocked','udp':'blocked','host':'blocked'})
            self.assertEqual(len(hits),0 if dead else 1)
            server.shutdown()
            ide.shutdown()
    def test_jsonc_settings_preserve_comments_custom_wrapper_and_other_keys(self):
        text='''{ // user preference\n "editor.fontSize": 17,\n "claudeCode.claudeProcessWrapper": "/home/james/custom launcher",\n}'''
        patched, previous = agent.patch_settings(text, "/home/james/nodecloak/claude-vscode")
        self.assertEqual(previous, "/home/james/custom launcher")
        self.assertIn("// user preference", patched)
        self.assertIn('"editor.fontSize": 17', patched)
        self.assertEqual(agent.patch_settings(patched, "/home/james/nodecloak/claude-vscode")[0], patched)
        restored = agent.restore_settings(patched, "/home/james/nodecloak/claude-vscode", previous)
        self.assertEqual(agent.jsonc(restored)["claudeCode.claudeProcessWrapper"], previous)
        self.assertEqual(agent.jsonc(restored)["editor.fontSize"], 17)

    def test_install_remove_preserves_shell_settings_and_detects_unsupported_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            home=Path(tmp); (home/".bashrc").write_text("# My settings\nexport EDITOR=vim\n")
            settings=home/".vscode-server/data/Machine/settings.json"
            settings.parent.mkdir(parents=True); settings.write_text('{"editor.fontSize":17}')
            agent.install(home, "/mnt/c/NodeCloak/nodecloak.exe", SOURCE.read_text())
            root=agent.root_for(home)
            self.assertTrue((root/"bin/codex").is_file())
            self.assertTrue((root/"bin/claude-vscode").is_file())
            self.assertEqual(agent.jsonc(settings.read_text())["editor.fontSize"],17)
            self.assertIn('nodecloak',agent.jsonc(settings.read_text())["claudeCode.claudeProcessWrapper"])
            agent.uninstall(home)
            self.assertEqual((home/".bashrc").read_text(),"# My settings\nexport EDITOR=vim\n")
            self.assertNotIn("claudeCode.claudeProcessWrapper",agent.jsonc(settings.read_text()))

    def test_protected_launch_fails_closed_when_configuration_is_missing_or_corrupt(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            for value in [None,"not json",'{"enabled":true,"apps":{}}']:
                if value is not None:(root/"state.json").write_text(value)
                proc=subprocess.run([sys.executable,str(SOURCE),"--host",str(root),"codex","--version"],capture_output=True,text=True,timeout=10)
                self.assertNotEqual(proc.returncode,0)
                self.assertIn("NodeCloak",proc.stderr)


if __name__ == "__main__":
    unittest.main()
