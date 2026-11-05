# Release secret scanning fixes and current master changes

- [ ] Finish and verify the complete secret redaction and popup changes.
- [ ] Commit and push the validated implementation with the approved identity.
- [ ] Merge all current master changes using signed no-ff merges; preserve concurrent work.
- [ ] Run the updated website/static validation and final local checks.
- [ ] Plan review: verify release scope includes the native transports, popup, JWT, Notify option and concurrent master updates.
- [ ] Test coverage review and bug hunt: verify tests, error paths, cancellation and publication boundaries.
- [ ] Dispatch release-validation CI for the exact master candidate SHA and monitor it.
- [ ] Cut the next unused stable version using the existing release workflow and monitor every platform to completion.
- [ ] Verify tag ancestry, release assets, updater signatures/platforms, automatic Docker/package delivery and website publication.
- [ ] Record results and limitations, then commit and push the verification record.

The owner explicitly authorized a new release after the fixes, including the other changes that reached master. This supersedes the earlier instruction not to release. No paid model calls or installed desktop app replacement are requested.

## Hosted validation follow-up

- [x] Inspect the first exact-SHA hosted run and distinguish test results from runner interruption.
- [x] Extend the Rust job timeout from 30 to 45 minutes so workspace tests, Clippy, formatting and cache upload can finish.
- [x] Plan review, test coverage review and bug hunt: verify this changes only the job time allowance and preserves all checks.
- [ ] Validate workflow syntax, commit/push the correction, merge using no-ff, then rerun exact-SHA validation.

Run 42063389661 at a461d8ee96768926a1f88dc0ba6cc71b0ece8f9e passed all Rust tests and all frontend/website checks. The Rust job reached its 30-minute limit during Clippy; formatting and cache upload were skipped. Local all-target Clippy and formatting had passed. This hosted run is not recorded as a green validation.

Workflow validation: actionlint passed; exact content comparison confirms the timeout is the sole workflow change.
