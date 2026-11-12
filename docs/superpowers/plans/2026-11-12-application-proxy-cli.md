# Windows CLI application proxy coverage

Scope: discover Claude Code native `.local/bin/claude.exe` and Codex native `.local/bin/codex.exe` plus npm's bundled Windows executables. Reuse existing claude/codex rule IDs and node selections. Never proxy a shared Node/shell runtime by executable name. Preserve desktop discovery and unrelated applications; macOS and WSL CLI support is outside this change.

- [x] Reproduce omissions with read-only local inventory and failing installation-layout tests.
- [x] Implement bounded fixed-layout CLI discovery and canonical running-path matching; verify dormant installations too.
- [ ] Verify discovered CLI paths resolve to assigned nodes and carry synthetic TCP/UDP on disposable Windows CI; preserve fail-closed regression checks.
- [ ] Update bilingual UI and documentation; run checks, review and integrate without restarting the active user's proxy or publishing a release.
