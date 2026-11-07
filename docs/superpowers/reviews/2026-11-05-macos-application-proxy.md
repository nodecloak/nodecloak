# macOS application proxy: engineering verification

Date: 2026-11-05. Tested source: 277971f6be505c89c500cc5557efb4ac993aa696; base: 9bfae7c.

Full matrix: https://github.com/nodecloak/nodecloak/actions/runs/41991173390 (all three jobs successful).

## Delivered behaviour

The shared Application Proxy UI supports Mac applications and administrator authorization. Application bundles expand into bounded, case-sensitive executable paths, including helpers. Nodes, credentials, optional first-hop proxy and diagnostics reuse the Windows model. Startup remains opt-in on every launch; browser-profile proxy handling is unchanged.

A transient launchd job owns the root helper and embedded core. Settings and credentials travel over authenticated IPC, not privileged files or command arguments. Peers must match their kernel audit token, expected UID/PID and live image hash. Both kernel queries bind to pidversion; invalid, debugged or killed code is rejected. Self strict verification, full protected-bundle verification and per-image hashes remain separate checks. Internal links stay inside the protected copy; absolute source-bundle load paths and external/broken links are rejected.

Platform references:

- Audit-bound read-only code queries: https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/kern_proc.c
- Socket peer identity: https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/uipc_usrreq.c
- Interface unit bound: https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/net/kpi_interface.c

## Executed checks

| Host | Shared tests and build | Native gate |
| --- | --- | --- |
| Windows | 232 frontend tests; 370 Rust tests passed, 52 default ignored; frontend and main-program builds passed | Existing real TUN TCP/UDP gate passed |
| macOS 15 ARM64 | 232 frontend tests; 358 Rust tests passed, 55 default ignored; frontend and main-program builds passed | Explicit native checks below passed |
| macOS 15 Intel | 232 frontend tests; 358 Rust tests passed, 55 default ignored; frontend and main-program builds passed | Explicit native checks below passed |

Ignored Mac checks were explicitly invoked on disposable CI hosts:

- Identity checks in both directions, including ordinary-user GUI to root helper in its private directory. Wrong hash, PID, UID, pidversion and a different executable were rejected.
- Packaged application with a nested executable copied and verified before root execution. AppleScript compiled without invoking an interactive authorization dialog.
- External symlink and source replacement rejected; unrelated sentinel file and network state preserved.
- Real TCP/UDP requests from three synthetic application processes, with the Claude probe inside a nested Helper.app. All six reached the assigned fixture; a same-name process at another path did not.
- Concurrent start rejected without disturbing the first session. Normal Stop, GUI EOF, core failure, authorization-launcher termination, helper termination, startup cancellation, unavailable node and occupied controller exercised.
- After each teardown, interfaces, stable routes, DNS/HTTP proxy preferences, protected-directory inventory and owned launchd-job absence checked against baseline.
- Pinned-core checks accepted HTTP, HTTPS, SOCKS5, VLESS, VMess and Hysteria2 for both configuration platforms.

Read-only official application layout checks on ARM64:

| Product | Observed version | Executable paths |
| --- | --- | --- |
| Codex / ChatGPT combined, com.openai.codex | 26.1029.70936 | 35 |
| ChatGPT classic, com.openai.chat | 1.2026.183 | 7 |
| Claude, com.anthropic.claudefordesktop | 1.0.2768 | 10 |

Official packages were inspected without installing or launching vendor applications. Publisher checks and actual helper expansion passed; source URLs and artifact digests are in the CI log. Claude uses a pinned official PKG fixture linked by its deployment documentation because the generic download redirect challenged CI requests.

## Review findings resolved

Independent review identified absolute source-bundle dependency paths, cancellation during scanning and late total-request budget checks. All were fixed and re-reviewed. Native testing additionally exposed whole-bundle relocation requirements, accepted-socket handshake ordering, inaccessible root-copy paths during user-side static peer verification, and out-of-range utun units. Negative tests, audit-bound kernel identity and interface boundary regression checks cover these cases.

Intel UI scheduling was limited to two workers after an existing five-second ProfileReset test timed out. No assertions or deadlines were relaxed.

## Coverage limits and release status

Routing uses synthetic processes, not real Codex/ChatGPT/Claude accounts or sign-in flows. Official bundle discovery and synthetic routing are separate evidence. Interactive administrator approval, real application sign-in, physical Wi-Fi changes and sleep/wake still require manual Mac acceptance; network/clock guard logic is unit tested. Native hosts ran macOS 15. Full native sessions used the packaged test app; the standalone test executable was checked for live audit identity, not a full elevated session.

The working Windows desktop and its network were not changed. No version bump, installer publication, website deployment or update-manifest change is included. Public 5.0.0 remains unchanged.
