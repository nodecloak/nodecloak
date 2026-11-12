# Application proxy fail-closed implementation plan

Goal: On Windows, applications assigned to a proxy must not regain external direct connectivity when the node, core, helper or GUI fails. Explicit Stop restores ordinary networking.

Architecture: Persistent WFP ALE filters match the selected executable paths and block non-loopback connections on every interface except the owned TUN. Install protection before starting the core; atomically update the allowed LUID after readiness. Unexpected termination leaves filters installed. Explicit Stop removes only NodeCloak filters, including recovery after restarting the GUI. IPv4 and IPv6 use equivalent policy. The helper owns privileged changes; the GUI only reads protection status.

Constraints: Preserve other applications and firewall policies. Preserve loopback IPC; applications using an independently configured local proxy remain outside the guarantee. Do not modify the active user's network for testing. macOS requires a separate enforcement implementation; do not claim equivalent protection there. No release requested in this task.

- [x] Add WFP engine, persistent sublayer, transactional replacement and recovery removal; compile and unit checks.
- [x] Integrate helper startup, explicit-stop versus EOF/crash lifecycle, and GUI status/recovery.
- [x] Add disposable Windows native regression checks for live TUN, node failure, helper/core failure, persistent protection, and explicit recovery; execute on CI.
- [x] Update UI copy, translations and application proxy documentation; run frontend checks and review final diff.

Verified implementation: 4b8900db85c2bcffb9ef674c359c3456fade9e77, GitHub run 43135916508. 44 backend tests and 13 frontend tests pass. Native tests confirm assigned TCP/UDP routing, dead-node failure, stop acknowledgement, core/helper/owner failure without direct TCP escape, unrelated-program connectivity, explicit recovery, and rejection of unknown process identity. IPv6 policy is installed but external IPv6 connectivity was not exercised. Local-only reproductions verified short/long WFP IDs differ and targeted console signalling preserves the helper. Independent static review completed. Current installed app is unchanged; no release performed.
