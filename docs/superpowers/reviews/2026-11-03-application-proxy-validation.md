# Windows application proxy validation — 2026-11-03

Implementation source: `61f858b3033525633c40d9e52740a6ab61058fd9`, branch `codex/application-proxy`. No release tag, published package or production configuration was changed for this feature.

## Verified

- [Windows CI 41589936498](https://github.com/nodecloak/nodecloak/actions/runs/41589936498): success. Frontend: 51 files / 231 tests passed. Rust library: 356 passed, 52 ignored; the relevant ignored native gates were then invoked explicitly and passed. TypeScript/Vite and the Windows executable build passed.
- Pinned sing-box 1.14.4 `check -c stdin`: generated HTTP, HTTPS, SOCKS5, VLESS, VMess and Hysteria2 application configurations accepted. No TUN starts during configuration checks.
- Real elevated Windows TUN integration, 25.03 seconds: six assigned flows observed (TCP and UDP from each of three disposable executables); another `ChatGPT.exe` in a different directory was not proxied. These are synthetic executables with application-like names, not the installed products.
- Normal Stop, owner pipe EOF, dead destination, cancellation during preparation and an occupied controller port passed. Each completed case compared the exact before/after routes, physical-adapter DNS and system HTTP proxy/PAC values. Owned interfaces/routes were checked as released.
- Existing browser relay regression tests passed locally using the isolated application build: VLESS and VMess transport matrix (32.32 seconds) and HY2 direct/UDP-chain/authentication/TLS cases (13.51 seconds). The browser helper's rejection of TUN and arbitrary configuration remains part of the unit suite.
- Local UI in headless Edge: add/edit node flow, rule assignment, explicit start/stop, disabled edits while running, no hover displacement, focus on the original 1px border, light/dark themes, 1440px and 1000px layouts. No page errors or horizontal overflow. Preview actions explicitly simulate routing.
- Read-only security review completed; actionable cancellation, teardown, path collision, Windows path alias, credential and VPN-preflight findings were fixed and rechecked. The reviewer did not claim independent native execution.

## Bugs found while validating

Windows can report the original 8.3 executable path or a long path. Generated rules now retain escaped exact aliases, including mixed component forms; application identity conflicts are checked separately from those aliases. Known detected-app version patterns remain bounded by product and publisher; manually selected executables keep exact identity.

The initial native SOCKS test server inherited nonblocking mode from its Windows listener and closed before the next handshake frame arrived. A standalone protocol test reproduced this without a TUN; explicitly making accepted sockets blocking fixed it. Diagnostic logs showed the initial TUN flow had already selected the assigned outbound, so this was not treated as evidence of a product routing failure.

One repeated local parallel full-suite run encountered pre-existing `Get-CimInstance Win32_Process` failures (`HRESULT 0x80004005`) in browser/profile tests. Its failure log was retained. No system service was restarted or test weakened; both the clean Windows runner and a local serial rerun (356 passed, 52 ignored, 245.45 seconds) passed the complete current suite. Feature-focused tests and the local browser transport regressions passed.

## Real applications — still incomplete

The current Windows desktop inventory found Codex desktop (`OpenAI.Codex`, whose executable is `ChatGPT.exe`) and the Codex CLI. It found no separate ChatGPT or Claude executable/package. Discovery correctly distinguished the Codex product from ChatGPT.

No claim is made that Codex, ChatGPT and Claude have all completed real application traffic, login or chat validation. The active user desktop was not put behind a TUN, existing application work was not force-closed, and no messages were sent. Installed-product network validation remains a separate outstanding check; the synthetic process matrix only establishes routing mechanics.

## User-visible behavior

Independent Windows sidebar page; off on each desktop start; local-only rules and separate OS credential storage; named destinations and optional first hop; explicit administrator authorization; live matching connection evidence and persistent redacted activity records. Stop is required before editing rules/nodes. Quit and updater preparation stop the owned session; hiding the main window retains background operation.

Unmatched applications go direct. System DNS and HTTP proxy settings are unchanged, while TUN temporarily changes routes. Application-specific proxies, loopback traffic and requests from shared hosts are not guaranteed to follow the selected executable rule. No per-application DNS privacy or platform-account availability guarantee is made.

Private evidence is under `.qa/application-proxy/`, including CI logs/result JSON, configuration checks, unit red/green logs, app inventory and UI screenshots.
