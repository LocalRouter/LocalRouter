# Monitor live duration

- [x] Inspect monitor event lifecycle and duration presentation.
- [x] Update list and detail durations while events are pending; stop on completion and unmount.
- [x] Verify live updates, final durations, and timestamp edge cases with regression tests.
- [x] Plan review: compare implementation to requested behavior and fill gaps.
- [x] Test coverage review: check pending, terminal, invalid, and future timestamps.
- [x] Bug hunt: inspect timer cleanup and state transitions.
- [x] Run frontend validation and required stable Rust workspace checks.
- [x] Commit and push validated changes and automatic catalog updates, preserving unrelated work.

## Implementation and review

A shared 100 ms clock updates only duration labels for visible pending events, across both the list and detail header. Subscribers release the clock on completion, failure, or unmount; the last subscriber clears the interval. Pending durations derive from the original event timestamp without mutating event data. Terminal durations use the backend value. Invalid timestamps retain the existing fallback; future timestamps clamp to zero.

Plan review, coverage review, and bug hunt found no remaining behavior gaps. Regression tests cover increasing elapsed time, completion and error transitions, fixed final durations, null durations, invalid timestamps, and future timestamps. Browser tests use a paused clock and explicitly deliver queued demo notifications.

Passed: frontend TypeScript, website typecheck, frontend production build, 10 monitor unit tests, 10 monitor browser tests, stable workspace Clippy, stable workspace formatting check, and git diff whitespace check. Stable toolchain verified as rustc 1.99.0. Full stable workspace tests passed: 3,686 tests passed, 94 ignored, no failures. Validation logs: `/private/tmp/monitor-duration-tests.log`, `/private/tmp/monitor-duration-clippy.log`, and `/private/tmp/monitor-duration-build.log`. Changes committed and pushed to the configured upstream on master.
