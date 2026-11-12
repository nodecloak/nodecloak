# Windows VS Code / WSL proxy verification

Scope: existing Codex and Claude application nodes cover Windows VS Code Claude native executables and explicitly connected WSL user sessions, without an external direct fallback for those protected sessions.

## Design review and corrections

- Independent review identified lost original VS Code wrapper settings after an interrupted installation. A failing interrupted-write test reproduced the loss. Integration metadata now journals original settings before any shell/editor mutation; retries and removal restore the original wrapper.
- The independent reviewer session became unavailable before completing its remaining review. The remaining implementation was reviewed inline against the implementation plan and exercised with the probes below; no full independent-review sign-off is claimed.
- A real WSL probe proved that clearing `WSL_INTEROP` does **not** prevent launching Windows network programs. The adapter now creates a private mount namespace, replaces that namespace's `/init` interpreter with `/usr/bin/false`, makes mounts private, drops capabilities and enables no-new-privileges. A regression probe invokes the actual Windows `cmd.exe` from inside the namespace and confirms failure.
- Stale adapter delivery after a desktop update was reproduced with an old executable script. State activation now refreshes the installed adapter atomically and updates the Windows relay path before publishing active state.
- A nonresponding relay remained alive after its tool exited. The adapter now tracks and terminates/reaps its own bridge processes during teardown. The regression uses a deliberately nonresponsive child and verifies that its PID is gone when the launcher returns.
- VS Code's explicit `CLAUDE_CODE_SSE_PORT` uses a separate exact loopback bridge. Unspecified host TCP services remain inaccessible from the namespace.

## Verified locally

- Rust app-proxy suite: 53 passed, six opt-in native checks excluded from the ordinary suite. Existing builtin app rules with manually selected Windows paths retain both their native adapters and the corresponding WSL assignment; custom applications are kept separate.
- WSL adapter suite: 14 tests, including the actual management protocol running from stdin in a temporary HOME, JSONC preservation, installation retry/removal, explicit Stop, exit/signal forwarding, capability drop, host/external IPv4/IPv6/UDP blocking, Windows interop denial and IDE-port forwarding. A real pseudo-terminal verifies that one Ctrl+C reaches the tool once and leaves its local proxy worker alive for continued use.
- Pinned Windows sing-box + WSL + actual NodeCloak stdio relay: separate Codex and Claude SOCKS fixtures each receive only their assigned application's request. Missing authentication is rejected. Desktop diagnostics identify the real `http/wsl-*` inbound metadata. Killing the core leaves subsequent protected requests blocked, without increasing either fixture's request count.
- Actual installed WSL Codex CLI 0.155.0 and Claude extension binary 2.1.322 start successfully with `--version` inside the protected namespace, without a reachable relay. This is startup compatibility evidence, not an authenticated service-login test.
- Frontend: 16 application-proxy tests; TypeScript/Vite production build succeeds.
- Headless browser: light and dark screenshots inspected, explicit connect/disconnect controls and English labels exercised, no page errors or horizontal overflow at desktop width. The synthetic preview makes no real desktop configuration changes.
- Local desktop debug executable compiled successfully. No user's running app, proxy selection, WSL home, host routes or system settings were changed by these tests; deployment tests used temporary homes and disposable fixtures.

Private local artifacts: `.qa/wsl-rust.log`, `.qa/wsl-agent.log`, `.qa/wsl-interop.log`, `.qa/wsl-build.log`, `.qa/wsl-proxy-light.png`, `.qa/wsl-proxy-dark.png`.

## Limits kept explicit

- Reopen terminals and extension sessions after integration or protection changes. Existing processes cannot be moved into a different network namespace retroactively.
- Direct absolute-path launches, root sessions, disconnected distributions and deliberate external IPC delegation are outside the WSL launcher boundary.
- Windows extension updates to a different executable path require stopping, rediscovering and restarting application proxy so the new path joins both routing and WFP protection.
- Other host loopback services, Windows executable helpers, privilege escalation inside protected tools and browser OAuth callback servers may need workflows outside the isolated session. The adapter never silently weakens isolation to make such a workflow succeed.

Final implementation `8234510daf64ccf4a40e0da1cb24c3aa394f4883` passed [Windows and Linux CI run 43145947005](https://github.com/nodecloak/nodecloak/actions/runs/43145947005). The disposable Windows runner additionally verified TCP/UDP routing for the native VS Code extension with an existing manually selected Claude executable, unknown-process rejection, core/helper/owner failure protection and deliberate Stop recovery. No package or version release was requested for this change.
