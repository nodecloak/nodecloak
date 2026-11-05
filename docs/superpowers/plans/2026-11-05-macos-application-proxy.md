# macOS Application Proxy Implementation Plan

> Execute inline, task by task, using test-driven development; request an independent review before merging.

**Goal:** Extend Application Proxy to macOS Apple Silicon and Intel with the existing node protocols, first-hop proxies, per-application selection and diagnostics.

**Architecture:** Reuse the typed settings, credentials, rule compiler and controller monitor. Add platform-specific application discovery and a private Unix control channel to a transient privileged launchd job. The helper runs only the embedded, verified core; launchd retains ownership of all descendants even if the helper dies. macOS HTTP proxy and DNS preferences are never written.

**Tech Stack:** Rust/Tokio, Darwin Unix credentials and Security framework, launchd/AppleScript administrator authorization, pinned sing-box, React/TypeScript.

**Spec:** User request in this conversation: implement the proposed macOS Application Proxy, retain existing browser proxy behavior, target Codex/ChatGPT/Claude and custom apps.

## Global constraints

- Default off; no routing before successful administrator authorization and IPC authentication.
- Preserve Windows semantics and tests. Unsupported platforms remain read-only.
- Node protocols and first-hop compatibility remain identical to the existing independent application proxy.
- Only exact canonical application executable paths match; case-sensitive Mac paths and nested helpers must not become Windows-style case-insensitive patterns.
- Never execute user-provided commands or core binaries as root. No credentials in process arguments or files; configuration uses bounded private frames and core stdin.
- The root bootstrap verifies a copied executable against the caller's live code identity before launch; Unix peer checks use kernel audit tokens, dynamic code validity and expected PID.
- Root-only temporary files contain programs and launchd metadata only. Teardown removes exact owned files and the empty directory, never unrelated routes/files/processes.
- launchd's default process-group ownership must remain enabled. The core must not create a detached session or process group.
- Stop, cancellation, GUI EOF, core exit and helper SIGKILL must release the owned utun/routes. Network changes fail closed with an actionable restart message.
- No changes to the user's running Windows routing. Real TUN tests run only on disposable CI hosts.
- This task does not publish a new product version.

## Task 1: Portable Mac application selection and configuration

**Files:** `src-tauri/src/app_proxy/macos_bundle.rs`, `config.rs`, `discovery.rs`, `store.rs`, `checks.rs`.

**Interfaces:** `macos_bundle::executables(&Path) -> Result<Vec<String>, String>` expands an existing canonical bundle into bounded executable Mach-O paths. `config::compile_for(Platform, ...)` emits OS-specific TUN/routing configuration; `compile(...)` selects the current OS. Existing Windows tests explicitly retain the Windows compiler.

- [ ] Add failing tests for distinct Codex/ChatGPT bundle identities, literal case-sensitive paths, nested helper paths, traversal/foreign-system targets and duplicate rules.
- [ ] Implement bounded no-symlink bundle inspection and platform-specific path validation; retain Windows path/alias behavior.
- [ ] Verify focused Rust tests and pinned-core configuration checks on both platforms.

## Task 2: Privileged Mac session and owned network lifecycle

**Files:** `app_proxy/macos/{mod,identity,transport,bootstrap,network,process,session}.rs`, `app_proxy/helper.rs`, `ipc.rs`, `runtime.rs`, `commands.rs`, `src-tauri/src/lib.rs`, `main.rs`, macOS Cargo dependencies.

**Interfaces:** Mac `Session::start(StartRequest, state, cancel, settings, rules, root)` / `close()` mirror the Windows session. The root entry accepts only a UUID suffix, expected parent PID and validated code identity. `network::snapshot/preflight/released` never mutates existing routes. The controller stream uses the existing bounded IPC codec.

- [ ] Add failing parser/bootstrap tests: shell metacharacters, invalid code identity, scoped Apple utun defaults, competing VPN defaults and colliding interface/subnet.
- [ ] Implement root-owned verified executable copy, per-session transient launchd registration and exact cleanup, with no shell interpolation of settings or credentials.
- [ ] Implement Unix audit-token authentication in both directions; reject incorrect UID/PID/code identity before reading secret configuration.
- [ ] Run the embedded core from a fresh protected directory, stream configuration over stdin, and validate readiness against the expected utun index/address and authenticated controller.
- [ ] Integrate cancellation, GUI EOF, graceful stop, core failure, helper crash ownership and network-change stop. Preserve diagnostic chronology and secret redaction.

## Task 3: Shared UI and documentation

**Files:** `src/ApplicationProxy.tsx`, `src/ApplicationProxy.test.tsx`, `src/application-proxy.ts`, `src/i18n.ts`, `docs/application-proxy.md`, `README.md`.

- [ ] Add a failing Mac view test asserting the feature can start, the selection/authorization copy is Mac-appropriate and no Windows-only restriction is shown.
- [ ] Reuse the existing VI layout and controls; expose the platform in the view, support `.app` selection and display the app bundle name.
- [ ] Describe authorization, stopping, supported protocols and application-helper/network-change limitations without package trust-status copy.
- [ ] Run the frontend tests, type/build checks and link checks appropriate to changed docs.

## Task 4: Native gates and review

**Files:** `app_proxy/macos/native_checks.rs`, `.github/workflows/application-proxy-checks.yml`, applicable build gates and private `.qa/macos-application-proxy/` evidence.

- [ ] On disposable ARM64 and Intel macOS runners, validate both peers' live identities and reject a different executable/PID before sending configuration.
- [ ] Exercise real TCP/UDP process routing, a same-name foreign process, nested helpers, normal Stop, GUI EOF, dead core, occupied controller, startup cancellation and helper SIGKILL.
- [ ] Compare interfaces, routes, HTTP proxy and DNS before/after each teardown. Verify no root job or core process remains.
- [ ] Verify the copied-helper path with a packaged application as well as the development/test executable.
- [ ] Re-run unchanged Windows native gates and shared frontend/Rust suites.
- [ ] Obtain independent security/correctness review, fix material findings, and record actual coverage and remaining real-app checks.
