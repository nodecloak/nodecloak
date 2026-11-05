"""Run real routing checks only on disposable Apple CI hosts, never a user's Mac."""
import json, os, pathlib, platform, subprocess, sys

assert platform.system() == 'Darwin'
assert os.environ.get('CI') == 'true'
assert os.environ.get('NODECLOAK_ALLOW_MAC_TUN_TESTS') == '1'
artifacts = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines()]
programs = [a['executable'] for a in artifacts if a.get('reason') == 'compiler-artifact' and a.get('profile', {}).get('test') and a.get('executable') and a.get('target', {}).get('name') == 'nodecloak_lib']
assert len(programs) == 1, programs
program = pathlib.Path(programs[0]).resolve()
subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(program)], check=True)
subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(program)], check=True)
for check in ['local_audit_tokens_reject_a_different_executable_before_config', 'native_macos_application_proxy_routes_and_recovers_owned_network']:
    subprocess.run([str(program), check, '--ignored', '--nocapture', '--test-threads=1'], check=True, timeout=600)
print('Mac process routing, peer identity and owned network teardown passed.')
