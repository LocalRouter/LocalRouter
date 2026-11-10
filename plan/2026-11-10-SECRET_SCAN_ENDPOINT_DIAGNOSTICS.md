- [x] Identify the warning emitter and inspect retained production metadata without accessing credentials or restarting production.
- [x] Replace inspection-mapping jargon with an unsupported-endpoint message containing method, host and path, excluding query and fragment values.
- [x] Test exact endpoint attribution, forwarded behavior, and diagnostic query privacy.
- [x] Plan review: check the requested wording and endpoint details are present.
- [x] Test coverage review: cover unsupported endpoints, known endpoints and sensitive URL query data.
- [x] Bug hunt: check forwarding behavior and information disclosure.
- [ ] Run relevant stable Rust checks and commit/push with the user identity, preserving concurrent work.
- [ ] Include the correction in a validated release and monitor publication/delivery to completion.

The existing v0.0.183 storage-recovery build uses an immutable tag. Do not modify that tag or claim this follow-up is included in its artifacts. Include the follow-up in the next release once the current release finishes. Do not change request blocking behavior, access the production credential store, or restart production.


Review: the old event does not retain a method/path, and passthrough events do not retain its request session. The production Monitor was inspected without restarting production or revealing/accessing stored credentials; the exact old endpoint could not be established reliably. The supplied plugin GETs cannot trigger this warning: the warning applies to suspected inference POSTs or realtime connections. No endpoint identity was guessed.

The warning now states the unsupported endpoint using method, host and path and explicitly says it was sent without scanning. Query and fragment values are excluded. A new Rust regression verifies the exact attribution, sensitive URL exclusion and continued forwarding. Warning details show the diagnostic directly and do not claim No secrets detected. Normal scan results retain their findings UI.

Local checks: 5,128 Rust tests passed, 84 ignored; all-target Clippy, formatting and debug build passed before the frontend-only detail edit. Both installed Codex and Claude Code passed exact restored replies, tool calls and files on this backend; isolated app stopped and test authentication copy removed. The actual warning detail component was rendered for protection and restoration warnings with message/outcome assertions and no false clean-scan result. TypeScript/desktop build, 150 frontend tests and website build/prerender/link checks passed. An attempted Playwright SSR test used Playwright JSX placeholders rather than React elements and was removed; a direct esbuild/React render check validated the real component instead. Evidence: /tmp/localrouter-storage-recovery/endpoint-validation.
