# VS Code Claude and WSL application proxy implementation plan

> **For agentic workers:** Execute inline, test-first; verify each boundary before integration.

**Goal:** Route Windows VS Code Claude and WSL Codex/Claude through the corresponding saved application node; failures must not enable direct external connections.

**Architecture:** Extend exact native executable discovery for the Windows extension. WSL integration installs user-owned CLI launchers and the documented Claude extension process wrapper. Each protected invocation creates an unprivileged user/network namespace with loopback only, drops capabilities and enables no-new-privileges before running the original command. A Unix-domain bridge outside the namespace forwards HTTP proxy traffic through Windows interop to authenticated, application-specific loopback inbounds on the existing sing-box process. There is no veth, host route, distro firewall, system proxy or system DNS change. Bridge, app, GUI and core failures cannot add an external interface to that namespace.

**Tech Stack:** Rust/Tokio, existing sing-box, Python 3 standard library, Linux unshare/ip/setpriv, React.

**Spec:** User request in this chat: local VS Code extension and WSL CLI coverage, reuse Codex/Claude nodes, no direct fallback.

## Global constraints

- Do not proxy all Code.exe, node.exe, wsl.exe or unrelated WSL applications.
- Integrate each WSL distribution explicitly in the application-proxy page; preserve and restore existing shell/VS Code settings, including JSONC comments and custom wrappers.
- Start protection before exposing a running session. Missing dependencies, invalid state, unsupported isolation or inaccessible bridge fail closed. Explicit Stop permits ordinary networking for subsequent launches; already isolated sessions remain isolated until restarted.
- Existing running terminals/extension sessions require a restart; never terminate user sessions automatically.
- Windows bridge inbounds authenticate, bind only 127.0.0.1 and route before process matching. No fallback outbound is allowed for their tags.
- Initial WSL coverage: Ubuntu with Python 3, iproute2, util-linux, user namespaces enabled. Other distributions receive capability checks and actionable errors rather than unsafe fallback.
- Namespace isolation blocks host TCP loopback services too; preserve VS Code IDE integration only through narrowly scoped loopback bridges where available. Do not claim arbitrary Windows interop/subprocess traffic is secured by a Linux network namespace.

## Tasks

- [x] Add regression tests for exact Windows VS Code extension discovery, including idle installations and rejecting unrelated/shared executables; implement and run.
- [x] Test and implement the WSL agent: safe JSONC integration and removal, real namespace proxying, external TCP/UDP/IPv6 denial with proxy variables bypassed, dead proxy, dead gateway, explicit Stop and signal/stdio preservation.
- [x] Add authenticated per-application inbounds and scoped Windows stdio relay, test node mapping/authentication and no direct fallback with the real pinned core.
- [x] Integrate WSL discovery/setup/removal and lifecycle with runtime. Preserve protection across failures and partially completed preparation, and show WSL connections in diagnostics.
- [x] Add a VI-aligned WSL integration section and tests for setup/removal, locked states and clear reconnect instructions. Document exact guarantees and limits.
- [ ] Run Rust/frontend/native/WSL tests; request independent code review; resolve findings and record verification. Packaging is not requested in this turn.
