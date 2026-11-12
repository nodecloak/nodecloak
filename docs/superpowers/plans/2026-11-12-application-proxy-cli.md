# Windows CLI application proxy coverage

Scope: discover Claude Code native `.local/bin/claude.exe` and Codex native `.local/bin/codex.exe` plus npm's bundled Windows executables. Reuse existing claude/codex rule IDs and node selections. Never proxy a shared Node/shell runtime by executable name. Preserve desktop discovery and unrelated applications; macOS and WSL CLI support is outside this change.

- [x] Reproduce omissions with read-only local inventory and failing installation-layout tests.
- [x] Implement bounded fixed-layout CLI discovery and canonical running-path matching; verify dormant installations too.
- [x] Verify discovered CLI paths resolve to assigned nodes and carry synthetic TCP/UDP on disposable Windows CI; preserve fail-closed regression checks.
- [x] Update bilingual UI and documentation; run checks, review and integrate without restarting the active user's proxy or publishing a release.

Verification: implementation 194df66d54cfdf9248c1dc010bdc817fe6c48a52, GitHub run 43137630008 succeeded. 46 backend tests, 13 frontend tests and frontend build pass. Local read-only inventory discovers actual native Claude Code and npm Codex CLI. Native Windows fixtures verify three install layouts, six TCP/UDP requests through discovered/resolved assigned nodes, unrelated Node exclusion, explicit Stop, and fail-closed regressions. Independent review passed after correcting pnpm sibling dependency discovery. Synthetic CLI traffic only; actual account/API sessions were not invoked. No local application restart or release performed.
