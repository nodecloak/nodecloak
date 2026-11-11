#!/usr/bin/env bash
set -euo pipefail
openbox >/tmp/nodecloak-test-openbox.log 2>&1 &
wm=$!
trap 'kill "$wm" 2>/dev/null || true' EXIT
printf '\n' | gnome-keyring-daemon --unlock --components=secrets
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo build --manifest-path src-tauri/Cargo.toml --locked
export NODECLOAK_TEST_FIREFOX_NATIVE_HOST="$PWD/src-tauri/target/debug/nodecloak"
export NODECLOAK_TEST_PROXY_HOST="$PWD/src-tauri/target/debug/nodecloak"
cargo test --manifest-path src-tauri/Cargo.toml --locked native_linux -- --ignored --nocapture --test-threads=1
