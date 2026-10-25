- [x] Reproduce Google issuer metadata mismatch and trace MCP creation errors.
- [x] Accept only Google's documented HTTPS root issuer slash discrepancy; preserve canonical metadata issuer and reject other mismatches.
- [x] Render MCP creation failures inside the modal, retaining form values and allowing retry.
- [x] Add issuer acceptance/rejection and modal error/retry regressions.
- [x] Plan Review: compare implementation with intended behavior.
- [x] Test Coverage Review: verify success, failures, canonical issuer and retry paths.
- [x] Bug Hunt: inspect security boundaries, form reset and retry state.
- [x] Run exact stable Rust CI checks, frontend checks and browser regressions.
- [x] Commit and push completed validated changes on the worktree branch, including automatic catalog updates.

Live evidence: Gmail protected-resource metadata advertises https://accounts.google.com/; Google's OpenID metadata returns issuer https://accounts.google.com. Keep the canonical metadata issuer for token binding and callback iss checks. Do not normalize arbitrary issuer URLs, paths, ports, queries or hosts.

The creation handler currently only emits a global toast. Put the failure inside the active modal, not in a background toast; clear on retry/reset and preserve the input values on failure. Scope is this creation flow, not a global notification redesign.


## Implementation review

Issuer validation now permits exactly the observed pair: advertised `https://accounts.google.com/` with metadata issuer `https://accounts.google.com`. Metadata's canonical issuer remains unchanged in the discovery result, preserving credential and callback issuer binding. Other hosts, paths, explicit ports, queries, fragments, insecure schemes and missing/mismatched issuers remain rejected. A local HTTP discovery regression checks that mismatched metadata still fails before its authorization/token endpoints are accepted.

MCP creation failures now appear as an alert inside the dialog, above the scrolling form, instead of an obscured global toast. Inputs survive failure. Error state clears on retry, Back and modal reset. Browser coverage verifies the alert is the top element at its on-screen center, credentials remain populated, Create becomes available again and retry/reopen resets work.

Separate authorized browser task completed: the personal Google project is now External with publishing status Testing. Confirmed saved test users: matus@smotana.com, matusfaro@gmail.com and matus.faro@upwave.com. The existing OAuth client/callback/secret are unchanged; no public publication or user-data authorization was performed.

Stable compiler 1.99.1 was updated and pinned using PATH/RUSTC; Clippy with warnings denied and formatting passed. Frontend/website builds and TypeScript passed. Full Rust workspace validation passed: 4,600 tests passed, 69 ignored, zero failures across 111 suites. All 96 frontend unit tests passed. The 15 existing/template browser regressions passed; the final creation-error regression passed with corrected detail-to-list navigation, including Back, retry and reopening reset. Both Gmail and Drive live resource metadata were confirmed to advertise the root-slash issuer alias. Installed production app remains unchanged; source fixes require a new build.
