# Monitor streamed content and stable event selection

- [x] Inspect supplied event shapes, capture reconstruction, summary previews, and selection lifecycle.
- [x] Reconstruct Codex Responses output from streamed items/deltas when terminal output is empty; preserve usage and completion metadata.
- [x] Preserve useful bounded question/answer content when parsed monitor bodies are truncated, and read captured raw payloads for existing events when needed.
- [x] Ignore null error fields when extracting answers.
- [x] Keep the detail split mounted while loading another event; handle rapid selections, missing details, and errors without stale content.
- [x] Add sanitized regression fixtures for lite Responses streams, oversized bodies, null errors, and delayed event selection.
- [x] Run targeted Rust/frontend/browser regressions and required stable workspace Clippy, formatting, and tests; verify builds.
- [x] Plan review: compare implementation to all requested behaviors and fill gaps.
- [x] Test coverage review: cover missing/error/partial/large payload and selection race paths.
- [x] Bug hunt: review reconstruction ordering, duplicate text, truncation budgets, and async state.
- [x] Commit and push validated task changes, including automatic catalog updates, preserving unrelated work.

Use an isolated worktree based on master. Samples are diagnostic data only; fixtures retain protocol shapes with synthetic text, IDs, and metadata. Raw wire capture remains available. Avoid changing traffic forwarded to providers or raising capture memory limits.

Implementation: assembled Responses items/text/tool arguments remain separate from the terminal envelope, with completed snapshots taking precedence over deltas. Truncation markers retain bounded semantic excerpts within the existing prefix budget. Detail display recovers legacy raw JSON/SSE without replacing original payloads. Selection preserves the resizable split; loading, missing events, failed loads/retry, and stale results are handled explicitly.

Review: added partial empty-content snapshot coverage after the bug hunt, plus multi-part ordering, tool arguments, incomplete/failed terminal envelopes, null errors, pending-to-complete summaries, and oversized metadata. Existing browser fixtures now select the intended event by content/ID rather than assuming the first row.

Validation: stable Rust 1.99.0 update, workspace Clippy with warnings denied, formatting check, and full workspace tests passed. Eight Monitor browser tests and eight presentation tests passed; app and website builds passed. Included the automatic model catalog refresh.
