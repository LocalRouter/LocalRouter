# Available Try It Out panels and compact navigation

- [x] Centralize panel availability using existing provider path/feature support, current mode, known client grants and allowed-model provider resolution. Unknown data remains visible; hide only confirmed unsupported or disallowed panels.
- [x] Keep common Chat/Images/Embeddings/Speech/Transcribe panels in the main row, move secondary panels to accessible More, retain a visible selected secondary tab, and select an available fallback when necessary.
- [x] Preserve native panels' independent model selection by aggregating eligible providers rather than restricting them to the selected chat model; scope async capability/permission results to their current selection.
- [x] Move Credential lifetime and review, Privacy profile, Provider search and web fetch, and Native provider operations below Danger Zone; clarify the descriptions and verify their order in the browser.
- [x] Validate availability, overflow selection, direct/wrapped modes, permission changes and unknown/failed lookups with focused regressions and a browser smoke; verify the running debug app hot reloads.
- [x] Plan Review: compare implementation with requested filtering and compact navigation, fix omissions.
- [x] Test Coverage Review: cover supported/unsupported/unknown states, mixed providers, provider name prefixes, stale scopes and fallback selection.
- [x] Bug Hunt: inspect async races, disappearing active tabs, dropdown keyboard/focus behavior and empty availability.
- [x] Run updated stable exact workspace Clippy/fmt/tests with pinned PATH/RUSTC, frontend regressions, app/demo types and builds.
- [x] After validation, commit/push to master and trigger the existing Release workflow for the next unused patch version, verifying dispatch and initial job status.
- [x] Commit and normal push validated task changes and automatic catalog updates; preserve unrelated release drafts and inspect CI.

Use existing read-only Tauri commands; no new provider calls, protocol changes or capability guesses. Local Files/Replay do not depend on a remote provider. Specialized native panels use eligible providers and client grants; capability lookup failures stay unknown. The UI uses the existing Radix dropdown and tab controls. The debug app remains running with --no-watch so frontend changes hot reload.

Plan Review, Test Coverage Review and Bug Hunt completed. Unknown/failed support
lookups remain visible; known unsupported paths and revoked operation/search
grants are hidden. Native panels aggregate eligible provider support instead of
using the selected chat model. Qualified provider names use longest-prefix
matching; ambiguous IDs remain unknown. Async support, permissions and model
responses are scoped to their current selection, and obsolete model requests
cannot replace a newer client's list. Image edits keep Images available even
when generation is unavailable. Selecting a secondary panel adds just that tab
to the common row; losing availability selects an eligible fallback.

All four requested settings cards follow Danger Zone in the requested order,
with concrete descriptions of their behavior. No Tauri command/schema changes,
paid provider calls, or backend changes were introduced.

Final validation on rustup stable 1.99.1 with confirmed/pinned PATH and RUSTC:
workspace Clippy (-D warnings), fmt and 111 test suites passed: 4,523 passed,
69 ignored, zero failures. All 82 frontend regressions passed, including nine
new filtering/navigation regressions. App and website type checks and production
builds passed. Production Chrome verified common-only initial tabs, More menu
keyboard access, selected overflow visibility, default-off and updated grants,
settings card order and a narrow viewport, without page errors or external
requests. Screenshot: /private/tmp/localrouter-compact-panels.png. Logs:
/tmp/try-it-out-{clippy,fmt,workspace}.log and
/tmp/try-it-out-{unit,app-types,website-types,app-build,website-build,browser}-final.log.

Delivery: normal commit/push to master, then the existing release.yml workflow
with version 0.0.160 and prerelease=false. GitHub's latest published version was
0.0.159 and v0.0.160 was unused at preparation. Dispatch/run status is reported
in the delivery response. The existing workflow owns version bumps and tags;
unrelated historical release drafts remain untouched.
