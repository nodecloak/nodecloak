"""Install and launch each Ubuntu package on a disposable CI desktop."""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

root = Path(__file__).resolve().parents[1]
version = json.loads((root / "package.json").read_text())["version"]
bundle = root / "src-tauri/target/release/bundle"
deb, = (bundle / "deb").glob("*.deb")
appimage, = (bundle / "appimage").glob("*.AppImage")
metadata = subprocess.check_output(["dpkg-deb", "-f", str(deb), "Version", "Architecture"], text=True)
assert f"Version: {version}" in metadata and "Architecture: amd64" in metadata, metadata
subprocess.run(["sudo", "dpkg", "-i", str(deb)], check=True)
assert Path("/usr/bin/nodecloak").is_file()
assert any("nodecloak" in p.read_text().lower() for p in Path("/usr/share/applications").glob("*.desktop"))
report = {"version": version, "architecture": "amd64", "packages": {}}
wm = subprocess.Popen(["openbox"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    for name, program in [("deb", Path("/usr/bin/nodecloak")), ("AppImage", appimage)]:
        with tempfile.TemporaryDirectory(prefix="nodecloak-package-") as directory:
            env = dict(os.environ, HOME=directory, XDG_CONFIG_HOME=directory + "/config",
                       XDG_DATA_HOME=directory + "/data", XDG_CACHE_HOME=directory + "/cache",
                       APPIMAGE_EXTRACT_AND_RUN="1", WEBKIT_DISABLE_DMABUF_RENDERER="1")
            with tempfile.TemporaryFile() as log:
                process = subprocess.Popen([str(program)], env=env, stdout=log, stderr=log, start_new_session=True)
                try:
                    deadline = time.monotonic() + 45
                    visible = False
                    while time.monotonic() < deadline and process.poll() is None:
                        windows = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "NodeCloak"], capture_output=True)
                        if windows.returncode == 0:
                            visible = True
                            break
                        time.sleep(0.25)
                    time.sleep(3)
                    log.seek(0)
                    diagnostic = log.read().decode(errors="replace")
                    assert visible and process.poll() is None, f"{name} failed to show its window: {diagnostic}"
                    report["packages"][name] = {"installedOrExtracted": True, "visibleWindow": True, "aliveAfterStartup": True}
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGTERM)
                        try:
                            process.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait(timeout=5)
finally:
    wm.terminate()
    wm.wait(timeout=5)
(root / "linux-package-verification.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
