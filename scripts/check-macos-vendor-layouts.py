"""Read official vendor DMGs without installing or launching their applications."""
import hashlib, json, os, pathlib, plistlib, subprocess, sys, tempfile, urllib.request

assert sys.platform == 'darwin' and os.environ.get('CI') == 'true'
program = pathlib.Path(sys.argv[1]).resolve()
sources = [
    ('openai', 'https://persistent.oaistatic.com/codex-app-prod/Codex.dmg', '2DC432GLL2'),
    ('anthropic', 'https://claude.ai/api/desktop/darwin/universal/dmg/latest/redirect', 'Q6L2SF6YDW'),
]
report = []
for vendor, url, expected_team in sources:
    with tempfile.TemporaryDirectory(prefix='nodecloak-vendor-layout-') as temporary:
        root = pathlib.Path(temporary).resolve()
        image = root / 'source.dmg'
        request = urllib.request.Request(url, headers={'User-Agent':'NodeCloak compatibility checks'})
        with urllib.request.urlopen(request, timeout=120) as response, image.open('wb') as output:
            assert response.geturl().startswith('https:')
            total = 0
            while chunk := response.read(1024 * 1024):
                total += len(chunk)
                assert total <= 1024 * 1024 * 1024
                output.write(chunk)
        mount = root / 'mounted'
        mount.mkdir()
        subprocess.run(['/usr/bin/hdiutil','attach','-readonly','-nobrowse','-mountpoint',str(mount),str(image)],stdin=subprocess.DEVNULL,check=True,timeout=120)
        try:
            applications = list(mount.glob('*.app'))
            assert applications, 'No application in official image'
            for app in applications:
                info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
                requirement = f'=anchor apple generic and certificate leaf[subject.OU] = "{expected_team}"'
                subprocess.run(['/usr/bin/codesign','--verify','--strict','-R',requirement,str(app)],check=True,timeout=120)
                env = dict(os.environ,NODECLOAK_MAC_VENDOR_BUNDLE=str(app))
                subprocess.run([str(program),'native_macos_vendor_bundle_layout','--ignored','--nocapture','--test-threads=1'],check=True,timeout=120,env=env)
                report.append({'vendor':vendor,'source':url,'imageSHA256':hashlib.sha256(image.read_bytes()).hexdigest(),'bundle':app.name,'identifier':info['CFBundleIdentifier'],'publisher':expected_team,'version':info.get('CFBundleShortVersionString')})
        finally:
            subprocess.run(['/usr/bin/hdiutil','detach',str(mount)],check=True,timeout=60)
pathlib.Path('.qa').mkdir(exist_ok=True)
pathlib.Path('.qa/macos-vendor-metadata.json').write_text(json.dumps(report,indent=2))
print(json.dumps(report,indent=2))
