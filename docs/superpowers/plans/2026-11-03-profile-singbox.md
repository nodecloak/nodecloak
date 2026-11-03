# Browser-profile sing-box integration

**Goal:** Run independent loopback-only sing-box instances for VLESS and VMess browser profiles, preserving the existing first-hop proxy order and browser isolation.

**Architecture:** Retain the Rust per-profile HTTP relay as the browser entry. Node modes use a private authenticated sing-box HTTP listener behind that relay. A NodeCloak helper owns each core, receives generated configuration over stdin, and shuts it down when the parent pipe closes. No TUN, system proxy, system DNS, global API listener, arbitrary user configuration or direct fallback.

**Tech stack:** Rust/Tauri, pinned sing-box 1.14.4 executable, React/TypeScript and existing VI.

**Constraints:** Preserve existing HTTP/HTTPS/SOCKS5, API extraction, credentials, cloud sync and received-share boundaries. Node UUIDs use the existing OS credential store, never plaintext profile JSON, command lines or logs. Cloud node configurations require a new portable schema so older clients reject safely. One running profile owns one core; tests and exit-IP probes use disposable cores. No production rollout/version bump requested.

- [x] Add failing parser/validation/config tests covering VLESS, VMess, TLS/Reality, TCP/WS/HTTPUpgrade/gRPC and rejection of unsupported options without leaking links.
- [x] Add typed node options, scoped UUID credentials and encrypted portable schema compatibility; preserve copy/edit/reset behavior and MCP validation.
- [x] Pin and hash-check upstream binaries/source/license, embed per target and include notices in release collection; make dev/build/CI preparation explicit.
- [x] Implement stdin-only configuration, private loopback listener, authenticated readiness, bounded port retries, parent-pipe lifecycle and per-profile process cleanup; no global process termination.
- [x] Verify real local VLESS/VMess servers, first-hop ordering, simultaneous isolation, failure closed, core/parent termination, unrelated core survival, no configuration secrets on disk and unchanged system network settings.
- [x] Add node-link import and read-only connection summary within the existing proxy editor; credentials masked, keyboard accessible, existing front-proxy UI retained; translate copy.
- [ ] Run targeted/full Rust/frontend/build checks, read-only code review, and platform CI/integration checks where available. Record actual platform limitations without claiming untested success.
