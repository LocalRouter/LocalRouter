- [ ] Complete and validate FAST_MCP_INITIALIZE in its isolated worktree; include automatic model catalog changes.
- [ ] Plan Review: preserve legacy configuration, permissions and fast-backend behavior; separate confirmed VocalCord outage from unproven latency attribution.
- [ ] Test Coverage Review: verify slow and failed upstreams, Skills read/search, list metadata, concurrent stateless requests, session cancellation and late recovery.
- [ ] Bug Hunt: inspect background ownership, notification deadline races, health recovery and credential redaction.
- [ ] Run pinned stable Clippy/fmt/workspace tests, commit and push the validated branch.
- [ ] Wait for v0.0.164 native publication, verify assets/updater manifest, then fetch current master with workflow version/Nix changes.
- [ ] Merge the fast-initialize branch into current master, validate the combined tree, commit and push, and await green CI.
- [ ] Verify v0.0.165 is unused and no competing release is running; review generated notes and dispatch the existing Release workflow with prerelease=false.
- [ ] Monitor all native builds through publication; verify release version, platform assets, matching updater signature files and tag ancestry. Check packaging followups separately.
- [ ] Report both published versions, the two-second foreground behavior and the remaining deleted-account remediation. Do not claim the installed application updated or current upstream latency was attributed without evidence.

The user explicitly requested one release now and another after the next fixes. This plan covers the second release. No production credentials/configuration changes, installed-app restart, issue comment or external message are part of this authorization.
