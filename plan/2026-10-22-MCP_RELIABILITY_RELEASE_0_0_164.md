- [x] Confirm latest published version is v0.0.163, v0.0.164 is unused, and no competing release is running.
- [x] Finish the independent MCP audit and reconcile any incident diagnostics that arrive; clearly distinguish confirmed gateway bugs from unverified external failures.
- [x] Plan Review: verify the MCP fix set against truthful health, Skills availability, authentication refresh and backwards compatibility; preserve unrelated work.
- [x] Test Coverage Review: verify failed/stale discovery, partial availability, session isolation, permission boundaries, local Skills and recovery cases.
- [x] Bug Hunt: review concurrency, cancellation, credential redaction and release state transitions.
- [x] Run stable CI-parity Clippy/fmt/workspace tests with pinned compiler; commit and push the validated worktree branch and automatic catalog updates.
- [x] Merge the validated branch into current master, retaining the other changes since v0.0.163. Validate the combined tree and commit/push the merge without force.
- [x] Review generated release notes for v0.0.164, verify master CI and resolve failures.
- [x] Dispatch the existing Release workflow on master for version 0.0.164, prerelease=false; let it bump and tag.
- [ ] Monitor all five native builds through actual publication, investigate failures, and verify release assets plus updater signatures/URLs.
- [ ] Report the published version, fixes and remaining incident limits; distinguish Nix/Docker/Flatpak/Snap follow-up status from native publication. Existing disabled store publishing remains disabled.

Authorization: the user explicitly requested a new release after completing this MCP reliability audit. Do not claim the installed application has updated unless verified. No additional issue comment or external message is authorized for this audit/release.

Audit worktree validation before commit: stable workspace Clippy/fmt and all 4577 tests passed (69 ignored); detailed findings and remaining evidence boundaries are in MCP_AVAILABILITY_AUDIT. The other queued master change is the secret-approval popup font-size fix.

Combined master validation passed: stable workspace Clippy with -D warnings, formatting, 4577 Rust tests across 111 suites (69 ignored), 90 frontend unit tests, both packaging regressions, app and website type checks/builds, and all ten secret-redaction browser cases using installed Chrome. The merge had no conflicts and preserves the queued popup-font change.

Release scope clarification: the user requested shipping this validated patch now and releasing the subsequent foreground-initialize deadline fix separately. v0.0.164 improves availability/health/auth/discovery, but does not yet guarantee initialize below Claude Code's reported 25-second timeout. That additional work is tracked in FAST_MCP_INITIALIZE in the audit worktree.

Master CI run 39828586464 passed both frontend/website and Rust gates. Release run 39830970395 was dispatched for 0.0.164, prerelease=false; version bump/tag 516cb817 is published to git. Native publication remains pending until all builds complete. Follow-up code is isolated from this tag and is validated separately for v0.0.165.
