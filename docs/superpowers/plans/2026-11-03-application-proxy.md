# Windows Application Proxy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an independent, default-off Windows TUN application proxy with per-application destinations and observable routing.

**Architecture:** Separate local settings/credentials, application discovery, strict configuration generation, privileged helper ownership and React workspace. Reuse existing proxy types and pinned core bytes, without extending the browser helper's permission to accept TUN configs. Execute inline and request independent review at the security and completion gates.

**Tech Stack:** Rust/Tauri 2, Windows named pipes/ShellExecute/Job objects, pinned sing-box 1.14.4, React/TypeScript and existing VI tokens.

**Spec:** `docs/superpowers/specs/2026-11-03-application-proxy.md`

## Global Constraints

- Windows first; unsupported systems expose an honest unavailable state.
- Off on first load and every desktop start. Starting requires an explicit action and OS administrator consent.
- Existing browser-profile proxies remain separate. No implicit global HTTP proxy or global DNS change.
- Runtime secrets travel only over restricted IPC and core stdin; settings and logs contain none.
- Retain system DNS; no per-application DNS privacy claim. Detect competing TUN/VPN routes.
- Never force-close the user's applications or send chats for validation. Distinguish available, absent and actually verified applications.

---

### Task 1: Local model, discovery and strict routing configuration

**Files:** Create `src-tauri/src/app_proxy/{mod.rs,model.rs,discovery.rs,config.rs,checks.rs}`; modify `src-tauri/src/lib.rs` only to register the module initially.

**Interfaces:** `Settings { version: u32, proxies: Vec<Destination>, rules: Vec<Rule> }`; `Rule { id, app, executable, proxy_id, enabled }`; `DetectedApp { id, name, paths, running }`; `ResolvedRule { id, name, paths, proxy_id }`. `compile(&[ResolvedRule], &[ResolvedProxy], controller_port, controller_secret) -> Result<Value,String>` validates and produces the entire fixed core config. `discover() -> Result<Vec<DetectedApp>,String>` discovers identities and paths without launching apps.

- [ ] Write tests that distinguish Codex's `OpenAI.Codex_.../ChatGPT.exe` from standalone ChatGPT; unknown `ChatGPT.exe` outside the known package is not auto-classified. Verify case/path normalization and known CLI matching.
  ```rust
  assert_eq!(identify(r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"), Some("codex"));
  assert_eq!(identify(r"C:\Tools\ChatGPT.exe"), None);
  ```
- [ ] Run the focused Rust tests red, implement path/application discovery and rerun green. Discovery includes running processes and bounded known install locations; no recursive scan of arbitrary user data.
- [ ] Add failing configuration tests: disabled/unassigned rules cannot enable a session; duplicate/conflicting executable matches fail; proxy secrets never enter persisted settings; unknown fields and unsupported API/system proxy modes fail.
  ```rust
  assert_eq!(config["route"]["final"], "direct");
  assert_eq!(config["inbounds"][0]["dns_mode"], "disabled");
  assert!(config["inbounds"][0]["auto_route"] == true);
  ```
- [ ] Implement typed validation, named destinations with a distinct keyring namespace and atomic local settings writes. Produce fixed outbound tags, localhost controller and protected process bypasses. Run generated fixtures through `sing-box check -c stdin`, including HTTP/SOCKS and all supported node types.

### Task 2: Owned elevated TUN helper and runtime

**Files:** Create `src-tauri/src/app_proxy/{runtime.rs,windows.rs,helper.rs,ipc.rs,native_checks.rs}`; modify `src-tauri/src/{main.rs,lib.rs,profile_commands.rs,proxy/core.rs}` and Windows dependency feature flags in `src-tauri/Cargo.toml`.

**Interfaces:** `Runtime::start(root, settings)`, `Runtime::stop()`, `Runtime::status()`; `run_helper(pipe_name, parent_pid)` executes before normal GUI/single-instance initialization. Retained process handles authenticate both pipe peers before a fresh challenge/response over the pipe; no token travels in argv. `proxy::core::embedded() -> (&'static [u8], &'static str)` shares only pinned compressed bytes/hash. Typed IPC uses a maximum 1 MiB length-prefixed JSON frame and authenticated peer identity.

- [ ] Write failing framing tests for oversized/truncated frames and wrong tokens; ensure bounded payloads and EOF produce shutdown. Add tests rejecting a raw arbitrary sing-box config at the elevated boundary.
  ```rust
  assert!(read_frame(&mut &b"\xff\xff\xff\xff"[..]).is_err());
  assert!(serde_json::from_value::<StartRequest>(serde_json::json!({"command":"cmd.exe"})).is_err());
  ```
- [ ] Implement named-pipe ACL/peer verification and the explicit runas launch. Detect cancellation and preserve inactive state. Serialize start/stop operations; report preparation/startup/stopping/error states without blocking unrelated browser operations.
- [ ] Implement fresh privileged extraction with SYSTEM/Administrators ownership and exact SHA256 verification. Launch only the embedded core with stdin configuration, an owned Job and hidden console/process group for graceful stop. No raw secret logs or arguments.
- [ ] Verify owned controller/interface readiness; collect bounded redacted connection evidence. Add bounded stop/owner-EOF handling and competing route-owner detection. Stop routing during application quit and updater preparation.
- [ ] Add ignored administrator-only native tests with disposable local targets: matched TCP/UDP routes through assigned fixtures, unmatched target remains direct, dead proxy never becomes direct, owner exit/startup error releases the owned interface and routes. Preserve the original browser helper's TUN rejection test.

### Task 3: VI workspace, rules and diagnostics

**Files:** Create `src/{ApplicationProxy.tsx,application-proxy.ts,application-proxy.css,ApplicationProxy.test.tsx,application-proxy.test.ts}`; modify `src/{App.tsx,i18n.ts}` and the native command registrations.

**Interfaces:** Bridge functions `getApplicationProxy`, `saveApplicationProxy`, `startApplicationProxy`, `stopApplicationProxy`, `discoverProxyApplications`, `chooseProxyExecutable`; view model contains settings, discovered applications, runtime status, redacted events/connections and platform support. Preview is explicitly simulated and never enables OS routing.

- [ ] Write failing React tests for initial inactive state, explicit start/stop, failed/cancelled start, rule assignment and unsupported Windows state; assert actual rendered actions/results using the real component with a narrow native transport double.
- [ ] Add the independent sidebar entry and page using existing VI colors, typography, buttons and focus treatment. Provide named destinations, detected app rows/custom executable picker, status and recent routing evidence. Keep edits disabled while a session runs; stop before applying changed rules.
- [ ] Use the existing proxy URI parser for pasted destinations; clear raw pasted credentials after save. Show missing applications and the difference between process matching and verified network traffic. Explain temporary routing/admin consent, unchanged DNS and existing application-proxy limitations in concise copy.
- [ ] Run component/bridge tests, TypeScript and the full frontend suite; inspect native/preview page at standard widths, keyboard focus and both themes. No hover displacement.

### Task 4: Integration, application checks and review

**Files:** Create private `.qa/application-proxy/` evidence and native test runner; update product docs with supported behavior and limits. Commit code only after required verification.

- [ ] Run Rust unit suite, generated core checks and existing browser-proxy tests. Build an isolated test executable without overwriting a running desktop binary.
- [ ] Run native TUN fixture checks with OS elevation, saving before/after routes/DNS/proxy state. If local consent is unavailable, run the same strict native gate on an authorized Windows CI runner; do not weaken assertions.
- [ ] Verify installed Codex, ChatGPT and Claude independently from actual process-path and connection evidence. Do not conflate Codex's ChatGPT.exe with the ChatGPT product. Record absent or unavailable apps honestly.
- [ ] Request read-only review of the privileged boundary, lifecycle, routing and UI; fix actionable findings, rerun affected checks and save exact test evidence. Leave routing off when verification ends and report the implementation and any remaining real-app validation limits.
