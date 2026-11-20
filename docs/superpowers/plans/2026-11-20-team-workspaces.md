# Team workspaces

NodeCloak 5.1.0 adds an account-backed Team page. An owner creates a workspace, invites an existing account by matching its profile name and email, and assigns selected browser configurations. Members accept an invitation and can run assigned copies. Member removal and leaving revoke all corresponding grants atomically.

- [x] Add the PostgreSQL team schema, membership API and team-scoped share authorization. Exercise ownership, invitation identity, independent direct shares and revocation with disposable PostgreSQL.
- [x] Add the authenticated Tauri bridge and the bilingual Team page. Reuse the existing encrypted share transfer and device authorization. Team assignments are configuration snapshots with system proxy and without website login data or personal notes. Validate account switching and member selection in the desktop UI.
- [ ] Verify frontend, Rust and Go suites. Build the server and all desktop targets, verify final packages and update signatures, and publish only after checks pass.
- [ ] Deploy the additive server schema and API, publish GitHub Release and update the website's six downloads, release notes and checksums. Verify public endpoints and downloads and keep rollback artifacts.
