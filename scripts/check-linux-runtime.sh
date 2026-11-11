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
for check in native_firefox_signed_fingerprint_launch_and_disable native_mcp_page_operations_all_browsers installed_browsers_large_session_roundtrip installed_node_transports_reach_only_local_fixture installed_hysteria2_transfers_direct_and_udp_chain_and_rejects_bad_auth_tls; do
    cargo test --manifest-path src-tauri/Cargo.toml --locked "$check" -- --ignored --nocapture --test-threads=1
done
