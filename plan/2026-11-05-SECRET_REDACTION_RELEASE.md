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
