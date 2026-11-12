"""Local-only pinned-core check: missing process identity must not route direct."""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

assert os.name == 'nt', 'Windows socket ownership fixture'
if len(sys.argv)>1 and sys.argv[1]=='share':
    with socket.create_connection(('127.0.0.1',int(sys.argv[3])),timeout=3) as client:
        print(client.share(int(sys.argv[2])).hex(),flush=True)
        sys.stdin.readline()
    sys.exit(0)

core = Path(os.environ['NODECLOAK_TEST_SINGBOX']).resolve()
assert core.is_file()
for guarded in [False, True]:
    with tempfile.TemporaryDirectory(prefix='nodecloak-unknown-process-') as directory, socket.socket() as target:
        target.bind(('127.0.0.1', 0))
        target.listen()
        target.settimeout(.5)
        with socket.socket() as allocation:
            allocation.bind(('127.0.0.1', 0))
            port = allocation.getsockname()[1]
        config = {
            'log': {'level': 'error'},
            'inbounds': [{'type': 'socks', 'listen': '127.0.0.1', 'listen_port': port}],
            'outbounds': [{'type': 'direct', 'tag': 'direct'}],
            'route': {'find_process': False, 'final': 'direct', 'rules':
                      [{'process_path_regex': ['.'], 'invert': True, 'action': 'reject'}] if guarded else []},
        }
        file = Path(directory) / 'fixture.json'
        file.write_text(json.dumps(config))
        with tempfile.TemporaryFile() as diagnostic:
            process = subprocess.Popen([str(core), 'run', '-c', str(file)], stdout=diagnostic, stderr=diagnostic,
                                       creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
            try:
                deadline = time.monotonic() + 10
                while True:
                    try:
                        ready = socket.create_connection(('127.0.0.1', port), timeout=1)
                        ready.close()
                        break
                    except OSError:
                        if process.poll() is not None or time.monotonic() >= deadline:
                            diagnostic.seek(0)
                            raise AssertionError(diagnostic.read().decode(errors='replace'))
                        time.sleep(.05)
                # Transfer the socket, then exit its creating process BEFORE
                # SOCKS negotiation. The flow survives but its original PID
                # cannot be resolved. find_process=false alone is insufficient:
                # sing-box automatically enables lookup for process rules.
                owner=subprocess.Popen([sys.executable,__file__,'share',str(os.getpid()),str(port)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True,creationflags=subprocess.CREATE_NO_WINDOW)
                try:
                    stream=socket.fromshare(bytes.fromhex(owner.stdout.readline().strip()))
                    stream.settimeout(3)
                    owner.stdin.write('transferred\n');owner.stdin.flush();owner.wait(timeout=5)
                    assert owner.returncode==0
                finally:
                    if owner.poll() is None:owner.kill();owner.wait(timeout=5)
                with stream:
                    stream.sendall(b'\x05\x01\x00')
                    assert stream.recv(2) == b'\x05\x00'
                    stream.sendall(b'\x05\x01\x00\x01\x7f\x00\x00\x01' + target.getsockname()[1].to_bytes(2, 'big'))
                    try:
                        response = stream.recv(32)
                    except OSError:
                        response = b''
                    if not guarded:
                        assert response[:2] == b'\x05\x00', response
                reached = False
                try:
                    peer, _ = target.accept()
                    peer.close()
                    reached = True
                except socket.timeout:
                    pass
                assert reached == (not guarded), f'unknown-identity direct fallback: guarded={guarded}, reached={reached}'
                print(f'unknown identity: guard={guarded}, direct target reached={reached}', flush=True)
            finally:
                process.kill()
                process.wait(timeout=10)
