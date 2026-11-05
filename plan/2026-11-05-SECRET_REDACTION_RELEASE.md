# Release secret scanning fixes and current master changes

- [x] Finish and verify the complete secret redaction and popup changes.
- [x] Commit and push the validated implementation with the approved identity.
- [x] Merge all current master changes using signed no-ff merges; preserve concurrent work.
- [x] Run the updated website/static validation and final local checks.
- [x] Plan review: verify release scope includes the native transports, popup, JWT, Notify option and concurrent master updates.
- [x] Test coverage review and bug hunt: verify tests, error paths, cancellation and publication boundaries.
- [x] Dispatch release-validation CI for the exact master candidate SHA and monitor it.
- [x] Cut the next unused stable version using the existing release workflow and monitor every platform to completion.
- [x] Verify tag ancestry, release assets, updater signatures/platforms, automatic Docker/package delivery and website publication.
- [x] Record results and limitations, then commit and push the verification record.

The owner explicitly authorized a new release after the fixes, including the other changes that reached master. This supersedes the earlier instruction not to release. No paid model calls or installed desktop app replacement are requested.

## Hosted validation follow-up

- [x] Inspect the first exact-SHA hosted run and distinguish test results from runner interruption.
- [x] Extend the Rust job timeout from 30 to 45 minutes so workspace tests, Clippy, formatting and cache upload can finish.
- [x] Plan review, test coverage review and bug hunt: verify this changes only the job time allowance and preserves all checks.
- [x] Validate workflow syntax, commit/push the correction, merge using no-ff, then rerun exact-SHA validation.

Run 42063389661 at a461d8ee96768926a1f88dc0ba6cc71b0ece8f9e passed all Rust tests and all frontend/website checks. The Rust job reached its 30-minute limit during Clippy; formatting and cache upload were skipped. Local all-target Clippy and formatting had passed. This hosted run is not recorded as a green validation.

Workflow validation: actionlint passed; exact content comparison confirms the timeout is the sole workflow change.

## Release candidate evidence

- Implementation f0e6ff020328dcbcd190dd056777de5f9f4bbaba; signed no-ff merge a461d8ee96768926a1f88dc0ba6cc71b0ece8f9e.
- Full validation at bcf6d6e412cffcff38e9e7252b9fea9c09b7e481 succeeded: CI run 42068889698.
- Included master push-relay changes d8c3a27a / 9bf9bc21. Repeated local validation: 236 server tests, all-target server Clippy, formatting, 148 frontend tests, desktop and website/static builds all passed. Hosted validation at 9bf9bc210c20d202a0558fe85e81adaa7040fe1e succeeded: CI run 42073420989.
- Included latest master Copilot token/toast fix 2ff092e728d92646a0c02c28d0d4df386e33c775. Local provider suite 731 passed (10 intentionally ignored), desktop all-target Clippy, formatting, 148 frontend tests, desktop and website/static builds, and all 49 popup browser tests passed.
- Final exact-source hosted validation at 2ff092e728d92646a0c02c28d0d4df386e33c775 succeeded: CI run 42076253791 (all source tests, lint, formatting, frontend and website checks green).
- Dispatched stable 0.0.175 release once: release run 42078890543. Tag 062c1416ac38a803a3f132ca2a76ba63b5061fd3 includes the validated source and differs only in Cargo.toml, Cargo.lock, package.json and src-tauri/tauri.conf.json version updates.
- Website deployments 42063248015 and 42070398806 succeeded; public research HTML was fetched and checked for the approved marker.
- Existing package-manager publication job is explicitly disabled (`if: false`, PACKAGING_PAT unavailable); do not claim Brew/Scoop/APT/YUM repository publication. Snap/Flatpak release builds, Nix pin and Docker are separate monitored jobs.

## Published artifacts verified

- Stable release published at https://github.com/LocalRouter/LocalRouter/releases/tag/v0.0.175, 2026-11-05T18:27:19Z.
- Downloaded the five updater artifacts and cryptographically verified every detached signature using minisign and the public updater key in tauri.conf.json. Both macOS bundles, both Linux AppImages and the Windows NSIS installer passed. Download sizes matched release asset metadata.
- Fetched the app's real `releases/latest/download/latest.json` endpoint; it serves v0.0.175 with all five expected platforms and the verified artifact URLs/signatures.
- Docker run 42084421111 succeeded. Independently fetched public GHCR manifests: tags 0.0.175, v0.0.175 and latest share digest sha256:52705edcc60d8e51a007e586e06c8ebca6795587f6a942cf04ed025a6d682495 and include Linux amd64 and arm64.
- Both Flatpak jobs and both Snap jobs succeeded. Nix pin update succeeded (84b9da14).
- Latest source website deployment 42075474653 (2ff092e7) succeeded. Fetched the public root and paper again, confirming the paper's placeholder text.

Limitations: native macOS credential storage was exercised with a fresh process; native Windows/Linux credential stores were not runtime-tested locally. Transport tests use synthetic upstreams; no paid hosted-model calls were made. Package-manager repository publishing remains explicitly disabled in the pre-existing workflow. The installed desktop application was not replaced or restarted.

## Final result

Release workflow 42078890543 completed successfully, including all five signed platform builds, both Snap builds, both Flatpak builds, GitHub publication and the Nix pin. Docker workflow 42084421111 also completed successfully. The latest-release API reports stable v0.0.175 with 32 nonempty assets, including both architectures of Snap and Flatpak. The package-manager repository job was skipped exactly as configured. No failed release jobs remain.

Final plan, test coverage and bug-hunt reviews are complete. All approved requirements have an implementation and verification record in the completeness plan. The only final changes are this documentation record and signed no-ff integration of the already published version/Nix commits. The final record is committed and pushed under Matus Faro's identity; unrelated root worktree files are preserved.
