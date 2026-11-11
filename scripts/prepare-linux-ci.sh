#!/usr/bin/env bash
set -euo pipefail
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev librsvg2-dev libayatana-appindicator3-dev libdbus-1-dev libfuse2t64 patchelf xvfb dbus-x11 gnome-keyring xterm xdotool openbox
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fL --retry 3 https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb -o "$work/chrome.deb"
curl -fL --retry 3 https://packages.microsoft.com/keys/microsoft.asc -o "$work/microsoft.asc"
gpg --dearmor < "$work/microsoft.asc" > "$work/microsoft.gpg"
sudo install -m 644 "$work/microsoft.gpg" /usr/share/keyrings/nodecloak-test-microsoft.gpg
echo 'deb [arch=amd64 signed-by=/usr/share/keyrings/nodecloak-test-microsoft.gpg] https://packages.microsoft.com/repos/edge stable main' | sudo tee /etc/apt/sources.list.d/nodecloak-test-edge.list
sudo apt-get update
sudo apt-get install -y "$work/chrome.deb" microsoft-edge-stable
curl -fL --retry 3 'https://download.mozilla.org/?product=firefox-latest-ssl&os=linux64&lang=en-US' -o "$work/firefox.tar.xz"
sudo tar -xf "$work/firefox.tar.xz" -C /opt
sudo ln -s /opt/firefox/firefox /usr/local/bin/firefox
google-chrome --version
microsoft-edge --version
firefox --version
