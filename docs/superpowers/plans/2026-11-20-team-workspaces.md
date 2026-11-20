# Team workspaces

NodeCloak 5.1.0 adds an account-backed Team page. An owner creates a workspace, invites an existing account by matching its profile name and email, and assigns selected browser configurations. Members accept an invitation and can run assigned copies. Member removal and leaving revoke all corresponding grants atomically.

- [x] Add the PostgreSQL team schema, membership API and team-scoped share authorization. Exercise ownership, invitation identity, independent direct shares and revocation with disposable PostgreSQL.
- [x] Add the authenticated Tauri bridge and the bilingual Team page. Reuse the existing encrypted share transfer and device authorization. Team assignments are configuration snapshots with system proxy and without website login data or personal notes. Validate account switching and member selection in the desktop UI.
- [x] Verify frontend, Rust and Go suites. Build the server and all desktop targets, verify final packages and update signatures, and publish only after checks pass.
- [x] Deploy the additive server schema and API, publish GitHub Release and update the website's six downloads, release notes and checksums. Verify public endpoints and downloads and keep rollback artifacts.

Local checks covered 246 frontend and 403 ordinary Rust tests, the full Go suite with isolated PostgreSQL, the native encrypted Firefox sharing flow, and the desktop Team workflow in a browser fixture. Release source `07420a16e3f4cf3841c69bf46b26d9bc6aae4415` passed the four-platform matrix in run `44753661049` after rerunning the failed jobs. The Ubuntu result was retained. The original Windows browser startup timeout passed on rerun, and both macOS jobs passed after the account holder resolved Apple's agreement requirement. Windows updater job `44763509127` and independent verification of all four updater signatures passed.

The account service passed schema migration 9, health, and access checks for teams and existing endpoints. The 5.1.0 stable GitHub Release was published after comparing all 15 final asset digests. Website commit `3b93bfe22ca022d19b9c3ca09069e4637bb25792` passed local checks and website run `44764171315`. The production switch verified 36 files and retained the prior static release.

Both official domains returned the full 5.1.0 download manifest. All six packages and ten static files were fetched over the official HTTPS endpoint and matched their expected hashes. The latest GitHub updater manifest and the production health and team authorization checks passed. The initial local public download check stalled on several ranged downloads; a fresh HTTPS download check from the deployment host verified every package. Private evidence is stored under `.qa/release-5.1.0/`.
