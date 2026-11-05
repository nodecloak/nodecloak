"""Read official vendor DMG/PKG files without installing or launching the apps."""
import hashlib, json, os, pathlib, plistlib, subprocess, sys, tempfile, urllib.request

assert sys.platform == 'darwin' and os.environ.get('CI') == 'true'
program = pathlib.Path(sys.argv[1]).resolve()
sources = [
    ('openai', 'https://persistent.oaistatic.com/codex-app-prod/Codex.dmg', '2DC432GLL2'),
    ('openai-classic', 'https://persistent.oaistatic.com/sidekick/public/ChatGPT_Desktop_public_latest.dmg', '2DC432GLL2'),
    # Official PKG linked by Claude's enterprise deployment documentation.
    # Pin this fixture; no installer scripts are executed by expand-full.
    ('anthropic', 'https://downloads.claude.ai/releases/darwin/universal/1.0.2768/Claude-21341c944a9d74880b52a77c4f1522f47f52d0da.pkg', 'Q6L2SF6YDW'),
]
report = []
for vendor, url, expected_team in sources:
    with tempfile.TemporaryDirectory(prefix='nodecloak-vendor-layout-') as temporary:
        root = pathlib.Path(temporary).resolve()
        is_pkg=url.endswith('.pkg')
        image = root / ('source.pkg' if is_pkg else 'source.dmg')
        request = urllib.request.Request(url, headers={'User-Agent':'NodeCloak compatibility checks'})
        with urllib.request.urlopen(request, timeout=120) as response, image.open('wb') as output:
            assert response.geturl().startswith('https:')
            total = 0
            while chunk := response.read(1024 * 1024):
                total += len(chunk)
                assert total <= 1024 * 1024 * 1024
                output.write(chunk)
        mount = root / 'mounted'
        if is_pkg:
            signature=subprocess.run(['/usr/sbin/pkgutil','--check-signature',str(image)],capture_output=True,text=True,check=True,timeout=60)
            assert expected_team in signature.stdout, 'Unexpected package publisher'
            subprocess.run(['/usr/sbin/pkgutil','--expand-full',str(image),str(mount)],check=True,timeout=180)
        else:
            mount.mkdir()
            subprocess.run(['/usr/bin/hdiutil','attach','-readonly','-nobrowse','-mountpoint',str(mount),str(image)],stdin=subprocess.DEVNULL,check=True,timeout=120)
        try:
            applications = [p for p in mount.rglob('*.app') if not p.is_symlink() and not any(a.suffix=='.app' for a in p.relative_to(mount).parents)]
            assert applications, 'No application in official image'
            for app in applications:
                info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
                entries=list(app.rglob('*'))
                print(json.dumps({'bundle':app.name,'entries':len(entries),'maxDepth':max(len(p.relative_to(app).parts) for p in entries)}),flush=True)
                requirement = f'=anchor apple generic and certificate leaf[subject.OU] = "{expected_team}"'
                subprocess.run(['/usr/bin/codesign','--verify','--strict','-R',requirement,str(app)],check=True,timeout=120)
                env = dict(os.environ,NODECLOAK_MAC_VENDOR_BUNDLE=str(app))
                subprocess.run([str(program),'native_macos_vendor_bundle_layout','--ignored','--nocapture','--test-threads=1'],check=True,timeout=120,env=env)
                report.append({'vendor':vendor,'source':url,'imageSHA256':hashlib.sha256(image.read_bytes()).hexdigest(),'bundle':app.name,'identifier':info['CFBundleIdentifier'],'publisher':expected_team,'version':info.get('CFBundleShortVersionString')})
        finally:
            if not is_pkg:
                subprocess.run(['/usr/bin/hdiutil','detach',str(mount)],check=True,timeout=60)
pathlib.Path('.qa').mkdir(exist_ok=True)
pathlib.Path('.qa/macos-vendor-metadata.json').write_text(json.dumps(report,indent=2))
print(json.dumps(report,indent=2))
