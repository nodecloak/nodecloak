#!/usr/bin/env python3
"""User-scoped WSL launchers. Protected tools have loopback and no external NIC.

Only the host-side Unix socket relay can invoke Windows interop. Neither routes,
DNS nor firewall rules in the distribution's original namespace are changed.
"""
import ctypes
import json
import os
from pathlib import Path
import re
import select
import shlex
import shutil
import signal
import socket
import socketserver
import subprocess
import sys
import tempfile
import threading
import fcntl
from contextlib import ExitStack

KEY = "claudeCode.claudeProcessWrapper"
MARKER = '# >>> nodecloak application proxy >>>\nexport PATH="$HOME/.local/share/nodecloak/application-proxy/bin:$PATH"\n# <<< nodecloak application proxy <<<\n'


def root_for(home):
    return Path(home) / ".local/share/nodecloak/application-proxy"


def jsonc_tokens(text):
    # Preserve offsets while removing only JSONC comments; strings remain intact.
    return re.sub(r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*[\s\S]*?\*/',
                  lambda m: m[0] if m[0].startswith('"') else re.sub(r'[^\n]', ' ', m[0]), text)


def jsonc(text):
    clean = jsonc_tokens(text)
    # Remove trailing commas outside quoted strings.
    clean = re.sub(r'"(?:\\.|[^"\\])*"|,(?=\s*[}\]])',
                   lambda m: m[0] if m[0].startswith('"') else ' ', clean)
    value = json.loads(clean)
    if not isinstance(value, dict):
        raise ValueError("VS Code settings must be an object")
    return value


def property_span(text, key):
    clean = jsonc_tokens(text)
    depth = 0
    tokens = re.finditer(r'"(?:\\.|[^"\\])*"|[{}\[\]]', clean)
    for token in tokens:
        value = token[0]
        if value.startswith('"'):
            if depth == 1 and json.loads(value) == key:
                tail = clean[token.end():]
                match = re.match(r'\s*:\s*("(?:\\.|[^"\\])*")', tail)
                if not match:
                    continue
                return token.end()+match.start(1), token.end()+match.end(1)
        elif value in ('{', '['):
            depth += 1
        else:
            depth -= 1
    return None


def patch_settings(text, wrapper):
    value = jsonc(text)
    previous = value.get(KEY)
    if previous == wrapper:
        return text, previous
    if previous is not None and not isinstance(previous, str):
        raise ValueError("Existing Claude process wrapper is not a string")
    span = property_span(text, KEY)
    if span:
        return text[:span[0]]+json.dumps(wrapper)+text[span[1]:], previous
    end = jsonc_tokens(text).rfind('}')
    prefix = text[:end]
    clean = jsonc_tokens(prefix).rstrip()
    separator = '' if clean.endswith(('{', ',')) else ','
    return prefix+separator+'\n  '+json.dumps(KEY)+': '+json.dumps(wrapper)+'\n'+text[end:], previous


def restore_settings(text, wrapper, previous):
    if jsonc(text).get(KEY) != wrapper:
        return text  # User changed this later: never overwrite their change.
    if previous is not None:
        return patch_settings(text, previous)[0]
    clean = jsonc_tokens(text)
    match = re.search(r'"claudeCode\.claudeProcessWrapper"\s*:\s*"(?:\\.|[^"\\])*"', clean)
    start, end = match.span()
    after = clean[end:]
    comma = re.match(r'\s*,', after)
    if comma:
        end += comma.end()
    else:
        before = clean[:start].rstrip()
        if before.endswith(','):
            start = len(before)-1
    result = text[:start]+text[end:]
    jsonc(result)
    return result


def atomic(path, text, mode=0o600):
    path = Path(path)
    if path.is_symlink():
        raise ValueError("Refusing symbolic-link configuration")
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(dir=path.parent, prefix=".nc-")
    try:
        os.fchmod(fd, mode)
        with os.fdopen(fd, 'w') as stream:
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def state_read(root):
    file = root / "state.json"
    if file.is_symlink() or file.stat().st_size > 65536:
        raise ValueError("Invalid WSL proxy state")
    state = json.loads(file.read_text())
    if not isinstance(state.get("enabled"), bool) or not isinstance(state.get("apps"), dict):
        raise ValueError("Invalid WSL proxy state")
    if state['enabled'] and not state['apps']:
        raise ValueError("Protected WSL state has no application assignments")
    return state


def preflight():
    if os.getuid() == 0:
        raise ValueError("WSL integration requires a regular Linux user")
    for name in ("unshare", "ip", "setpriv", "python3", "mount"):
        if not shutil.which(name):
            raise ValueError("WSL 缺少 " + name + "；请安装 python3、iproute2、util-linux")
    subprocess.run(["unshare", "--user", "--map-current-user", "--keep-caps", "--net", "--mount",
                    "sh", "-c", "mount --make-rprivate / && if [ -e /init ]; then mount --bind /usr/bin/false /init; fi && ip link set lo up"], check=True, capture_output=True, timeout=10)


def install(home, windows_exe, source):
    root = root_for(home)
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(root, 0o700)
    wrapper = str(root/"bin/claude-vscode")
    metadata_path = root/"integration.json"
    metadata = json.loads(metadata_path.read_text()) if metadata_path.exists() else {"settings": {}, "shells": {}}
    metadata.setdefault('shells', {})
    edits = []
    for channel in (".vscode-server", ".vscode-server-insiders"):
        folder = Path(home)/channel
        if not folder.is_dir():
            continue
        file = folder/"data/Machine/settings.json"
        old = file.read_text() if file.exists() else '{}\n'
        new, previous = patch_settings(old, wrapper)
        if previous != wrapper:
            metadata["settings"][str(file)] = previous
        edits.append((file, new))
    for file in (Path(home)/".bashrc", Path(home)/".zshrc", Path(home)/".profile"):
        old = file.read_text() if file.exists() else ''
        if MARKER not in old:
            addition=('\n' if old and not old.endswith('\n') else '')+MARKER
            metadata['shells'][str(file)]=addition
            edits.append((file, old+addition))
    # Validate all settings before writing any shell hooks.
    atomic(root/"agent.py", source, 0o700)
    for app in ("codex", "claude", "claude-vscode"):
        script = '#!/bin/sh\nexec python3 '+shlex.quote(str(root/"agent.py"))+' --host '+shlex.quote(str(root))+' '+app+' "$@"\n'
        atomic(root/"bin"/app, script, 0o700)
    state=state_read(root) if (root/"state.json").exists() else {"enabled": False, "apps": {}}
    if state['enabled']:
        raise ValueError('请先停止应用代理，再重新接入 WSL')
    state['windowsExe']=windows_exe
    atomic(root/"state.json", json.dumps(state))
    # Journal before mutating settings. A retry must retain the original wrapper
    # even if the first attempt stopped after patching only one file.
    atomic(metadata_path, json.dumps(metadata))
    for file, text in edits:
        atomic(file, text, file.stat().st_mode & 0o777 if file.exists() else 0o600)
    return {"home": str(home), "installed": True}


def uninstall(home):
    root = root_for(home)
    if not (root/'state.json').exists() and not (root/'integration.json').exists():
        return  # Preflight failed before any hooks were installed.
    state = state_read(root)
    if state["enabled"]:
        raise ValueError("请先停止应用代理，再移除 WSL 接入")
    metadata = json.loads((root/"integration.json").read_text()) if (root/"integration.json").exists() else {'settings':{}}
    wrapper = str(root/"bin/claude-vscode")
    for name, previous in metadata["settings"].items():
        file = Path(name)
        if file.exists():
            atomic(file, restore_settings(file.read_text(), wrapper, previous), file.stat().st_mode & 0o777)
    for file in (Path(home)/".bashrc", Path(home)/".zshrc", Path(home)/".profile"):
        if file.exists():
            addition=metadata.get('shells', {}).get(str(file), MARKER)
            atomic(file, file.read_text().replace(addition, ''), file.stat().st_mode & 0o777)
    # Leave the inactive launchers available to already-open shells. Removing
    # their files would turn a harmless stale PATH entry into a broken command.
    (root/"integration.json").unlink(missing_ok=True)


def real_command(root, app, args, metadata):
    paths = [part for part in os.environ.get('PATH', '').split(os.pathsep)
             if Path(part).resolve() != (root/"bin").resolve()]
    if app == "claude-vscode":
        original = None
        channel='.vscode-server-insiders' if args and '/.vscode-server-insiders/' in args[0] else '.vscode-server'
        for name, previous in metadata.get("settings", {}).items():
            if name.startswith(str(Path.home()/channel)+'/') and previous:
                original = previous
                break
        if original:
            program=shutil.which(original,path=os.pathsep.join(paths))
            if not program or Path(program).resolve().parent == (root/'bin').resolve():
                raise ValueError('原来的 Claude 自定义启动器不可用；请修正后重新接入')
            return [program]+args
        if not args or not os.path.isabs(args[0]) or not args[0].endswith('/resources/native-binary/claude'):
            raise ValueError("Claude 扩展没有传入原生程序路径；请更新扩展")
        return args
    program = shutil.which(app, path=os.pathsep.join(paths))
    if not program or Path(program).resolve() == (root/"bin"/app).resolve():
        raise ValueError("未找到原来的 "+app+" CLI；请先在 WSL 中安装")
    return [program]+args


def copy_sockets(a, b):
    # Preserve half-closes; closing either input must not discard the response.
    inputs = [a, b]
    while inputs:
        readable, _, _ = select.select(inputs, [], [], 60)
        for source in readable:
            other = b if source is a else a
            data = source.recv(65536)
            if data:
                other.sendall(data)
            else:
                inputs.remove(source)
                other.shutdown(socket.SHUT_WR)


class ThreadedUnix(socketserver.ThreadingUnixStreamServer):
    daemon_threads = True
    request_queue_size = 64


def host(root, app, args):
    state = state_read(root)  # Never fall back if state is missing/corrupt.
    metadata = json.loads((root/"integration.json").read_text()) if (root/"integration.json").exists() else {}
    command = real_command(root, app, args, metadata)
    app = 'claude' if app == 'claude-vscode' else app
    if not state["enabled"] or app not in state["apps"]:
        os.execvpe(command[0], command, os.environ)
    endpoint = state["apps"][app]
    if not (isinstance(endpoint.get('port'), int) and 0 < endpoint['port'] < 65536
            and re.fullmatch('[a-f0-9]{64}', endpoint.get('password', ''))):
        raise ValueError("WSL 应用代理配置无效；已阻止直连")
    with tempfile.TemporaryDirectory(prefix="nc-wsl-") as temp:
        address = temp+'/proxy.sock'
        limit = threading.BoundedSemaphore(64)

        class Relay(socketserver.BaseRequestHandler):
            def handle(self):
                if not limit.acquire(blocking=False):
                    return
                proc = None
                try:
                    proc = subprocess.Popen([state["windowsExe"], "--nodecloak-wsl-relay", str(endpoint['port'])], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
                    def upload():
                        try:
                            while data := self.request.recv(65536):
                                proc.stdin.write(data)
                                proc.stdin.flush()
                        except (OSError, BrokenPipeError):
                            pass
                        finally:
                            proc.stdin.close()
                    thread = threading.Thread(target=upload, daemon=True)
                    thread.start()
                    while data := proc.stdout.read1(65536):
                        self.request.sendall(data)
                    self.request.shutdown(socket.SHUT_WR)
                except OSError:
                    pass
                finally:
                    if proc is not None:
                        proc.terminate()
                        try:
                            proc.wait(timeout=3)
                        except subprocess.TimeoutExpired:
                            proc.kill(); proc.wait()
                    limit.release()

        with ExitStack() as stack:
            server=stack.enter_context(ThreadedUnix(address, Relay))
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            stack.callback(server.shutdown)
            ide=None
            port_text=os.environ.get('CLAUDE_CODE_SSE_PORT','')
            if app=='claude' and port_text.isdecimal() and 1024<=int(port_text)<=65535:
                port=int(port_text)
                class Ide(socketserver.BaseRequestHandler):
                    def handle(self):
                        try:
                            with socket.create_connection(('127.0.0.1',port),timeout=3) as remote:
                                remote.settimeout(None)
                                copy_sockets(self.request,remote)
                        except OSError:
                            pass
                ide_address=temp+'/ide.sock'
                ide_server=stack.enter_context(ThreadedUnix(ide_address,Ide))
                threading.Thread(target=ide_server.serve_forever,daemon=True).start()
                stack.callback(ide_server.shutdown)
                ide={'port':port,'socket':ide_address}
            env = dict(os.environ)
            # The namespace child may not start Windows network programs through
            # the normal WSL interop environment; the host relay owns interop.
            env.pop('WSLENV', None)
            env['WSL_INTEROP'] = '/nodecloak/interop-disabled'
            payload = json.dumps({"socket": address, "password": endpoint['password'], "command": command, "ide":ide})
            invocation = ["unshare", "--user", "--map-current-user", "--keep-caps", "--net", "--mount",
                          sys.executable, str(root/"agent.py"), "--inside", payload]
            child=subprocess.Popen(invocation,env=env)
            handlers={}
            for sig in (signal.SIGINT,signal.SIGTERM,signal.SIGHUP):
                handlers[sig]=signal.signal(sig,lambda number,frame: child.send_signal(number) if child.poll() is None else None)
            try:
                result=child.wait()
                return 128-result if result<0 else result
            finally:
                for sig,handler in handlers.items():signal.signal(sig,handler)
                if child.poll() is None:
                    child.terminate();child.wait()


def inside(payload):
    subprocess.run(["mount", "--make-rprivate", "/"], check=True, capture_output=True)
    # Clearing WSL_INTEROP alone is insufficient: /init also discovers ancestor
    # interop sockets. Shadow only this mount namespace's binfmt interpreter.
    if Path('/init').exists():
        subprocess.run(["mount", "--bind", "/usr/bin/false", "/init"], check=True, capture_output=True)
    # This operation can only affect the newly-created network namespace.
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True, capture_output=True)
    # Drop all namespace capabilities before any proxy worker or application.
    command = ["setpriv", "--no-new-privs", "--bounding-set=-all", "--inh-caps=-all", "--ambient-caps=-all",
               sys.executable, __file__, "--isolated", json.dumps(payload)]
    os.execv(command[0] if '/' in command[0] else shutil.which(command[0]), command)


def isolated(payload):
    def bridge(address):
        class Bridge(socketserver.BaseRequestHandler):
            def handle(self):
                try:
                    with socket.socket(socket.AF_UNIX) as remote:
                        remote.connect(address)
                        copy_sockets(self.request,remote)
                except OSError:
                    pass
        return Bridge
    class Proxy(socketserver.BaseRequestHandler):
        def handle(self):
            try:
                with socket.socket(socket.AF_UNIX) as remote:
                    remote.connect(payload['socket'])
                    copy_sockets(self.request, remote)
            except OSError:
                pass

    server = socketserver.ThreadingTCPServer(('127.0.0.1', 0), Proxy)
    server.daemon_threads = True
    port = server.server_address[1]
    ide=payload.get('ide')
    ide_server=None
    if ide:
        ide_server=socketserver.ThreadingTCPServer(('127.0.0.1',ide['port']),bridge(ide['socket']))
        ide_server.daemon_threads=True
    # The proxy worker follows the tool's lifetime and never has an external NIC.
    parent = os.getpid()
    worker = os.fork()
    if worker == 0:
        libc = ctypes.CDLL(None)
        libc.prctl(1, signal.SIGTERM, 0, 0, 0)  # PR_SET_PDEATHSIG
        if os.getppid() != parent:
            os._exit(1)
        if ide_server:
            threading.Thread(target=ide_server.serve_forever,daemon=True).start()
        server.serve_forever()
        os._exit(0)
    server.server_close()
    if ide_server:ide_server.server_close()
    env = dict(os.environ)
    for key in list(env):
        if key.lower() in ('http_proxy', 'https_proxy', 'all_proxy', 'no_proxy'):
            del env[key]
    proxy = f"http://nodecloak:{payload['password']}@127.0.0.1:{port}"
    for key in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy', 'https_proxy', 'all_proxy'):
        env[key] = proxy
    env['NO_PROXY'] = env['no_proxy'] = '127.0.0.1,localhost,::1'
    os.execvpe(payload['command'][0], payload['command'], env)


def main():
    if sys.argv[1:2] == ['--host']:
        return host(Path(sys.argv[2]), sys.argv[3], sys.argv[4:])
    if sys.argv[1:2] == ['--inside']:
        inside(json.loads(sys.argv[2]))
    elif sys.argv[1:2] == ['--isolated']:
        isolated(json.loads(sys.argv[2]))
    elif sys.argv[1:2] == ['--manage']:
        payload = json.loads(sys.argv[3])
        home = Path.home()
        root=root_for(home)
        root.mkdir(parents=True,exist_ok=True,mode=0o700)
        lock=open(root/'manage.lock','a')
        fcntl.flock(lock,fcntl.LOCK_EX)
        action = sys.argv[2]
        if action == 'install':
            preflight()
            windows_exe = subprocess.check_output(['wslpath', '-u', payload['windowsExe']], text=True).strip()
            result = install(home, windows_exe, globals()['source'])
        elif action == 'state':
            root = root_for(home)
            if not (root/'integration.json').is_file():
                raise ValueError('WSL 接入配置已移除，请重新接入')
            old = state_read(root)
            old.update(payload)
            atomic(root/'state.json', json.dumps(old))
            result = {'updated': True}
        elif action == 'remove':
            uninstall(home); result = {'installed': False}
        else:
            raise ValueError('Unsupported WSL action')
        print(json.dumps(result))
    else:
        raise ValueError('Invalid WSL proxy invocation')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError, KeyError) as error:
        print('NodeCloak: '+str(error)+'；未切换为直连', file=sys.stderr)
        sys.exit(1)
