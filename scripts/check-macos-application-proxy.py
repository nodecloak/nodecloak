"""Run real routing checks only on disposable Apple CI hosts, never a user's Mac."""
import json, os, pathlib, platform, plistlib, shutil, subprocess, sys, tempfile

assert platform.system() == 'Darwin'
assert os.environ.get('CI') == 'true'
assert os.environ.get('NODECLOAK_ALLOW_MAC_TUN_TESTS') == '1'
artifacts = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines()]
programs = [a['executable'] for a in artifacts if a.get('reason') == 'compiler-artifact' and a.get('profile', {}).get('test') and a.get('executable') and a.get('target', {}).get('name') == 'nodecloak_lib']
assert len(programs) == 1, programs
program = pathlib.Path(programs[0]).resolve()
subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(program)], check=True)
subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(program)], check=True)
subprocess.run([str(program), 'local_audit_tokens_reject_a_different_executable_before_config', '--ignored', '--nocapture', '--test-threads=1'], check=True, timeout=60)
if os.environ.get('NODECLOAK_VERIFY_VENDOR_LAYOUTS') == '1':
    subprocess.run([sys.executable,'scripts/check-macos-vendor-layouts.py',str(program)],check=True,timeout=900)
with tempfile.TemporaryDirectory(prefix='nodecloak-native-package-') as temporary:
    bundle = pathlib.Path(temporary).resolve() / 'NodeCloak Native Checks.app'
    executable = bundle / 'Contents/MacOS/native-checks'
    executable.parent.mkdir(parents=True)
    shutil.copy2(program, executable)
    (bundle / 'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier':'com.nodecloak.native-checks','CFBundleExecutable':'native-checks','CFBundlePackageType':'APPL','CFBundleVersion':'1'}))
    nested = bundle / 'Contents/Helpers/tool'
    nested.parent.mkdir()
    shutil.copy2('/usr/bin/true',nested)
    subprocess.run(['/usr/bin/codesign','--force','--sign','-',str(nested)],check=True)
    subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(bundle)], check=True)
    subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(bundle)], check=True)
    for check in ['local_audit_tokens_reject_a_different_executable_before_config', 'native_macos_application_proxy_routes_and_recovers_owned_network']:
        subprocess.run([str(executable), check, '--ignored', '--nocapture', '--test-threads=1'], check=True, timeout=600)
print('Mac process routing, peer identity and owned network teardown passed.')
