# Consistent dashboard traffic gaps

- [x] Trace dashboard series, zero-gap rendering, and live in-progress behavior.
- [x] Share idle-value and curve segmentation rules across LLM, MCP, and errors; preserve raw tooltip counts and live segments.
- [x] Add regression coverage for isolated spikes, short/long idle gaps, unknown buckets, boundaries, and in-progress requests.
- [x] Plan Review: compare all intended behaviors against implementation.
- [x] Test Coverage Review: cover modified paths and edge cases.
- [x] Bug Hunt: re-read rendering, tooltip and live transitions.
- [x] Run validation: TypeScript and chart regressions passed; required stable Rust checks attempted, with unrelated concurrent-work blockers documented below.
- [ ] Commit and push validated task changes and automatic catalog updates, preserving unrelated work.

## Review and validation

LLM and MCP previously passed raw zero counts to ordinary Recharts curves; errors alone hid idle buckets and split consecutive baseline zeros into separate paths. All three now use the same adjacent-zero selection and TrafficCurve renderer. Raw bucket counts remain available for tooltips and live in-progress segments. Tooltip null filtering is disabled so all-idle and unrecorded buckets remain inspectable. Hover markers appear only for nonzero activity, consistently across series.

Plan review, test coverage review and bug hunt completed. Per-series unit cases cover empty/single-point windows, isolated spikes, shared-zero and consecutive-zero gaps, longer idle gaps, window edges, unrecorded buckets and original count preservation. Live LLM and MCP requests retain their provisional segments on otherwise idle timelines. Browser regression checks actual separate SVG paths for both short and long gaps, rise/fall around single spikes, no idle paths, and zero tooltips.

TypeScript passed; 19 dashboard unit tests passed. Dashboard browser suite: 13 passed, including all traffic behavior checks; the unrelated monitor layout assertion failed (bottom 1180px vs 1081px limit). The identical assertion and dimensions reproduce on the unchanged fa79054c baseline in an isolated checkout.

Stable toolchain updated/confirmed at 1.99.1, with PATH and RUSTC pinned to rustup's compiler. Workspace Clippy passed. Workspace formatting failed only on unrelated concurrent secret-redaction edits (rechecked with the same result). Workspace tests could not compile lr-mcp because the concurrent FirewallApprovalAction::RedactOnce addition lacks a match arm in gateway/gateway_tools.rs:815. These files are outside this task and are preserved. No Rust code or Tauri command contracts are changed by the chart fix. Upstream's Nix source pin was fast-forwarded before committing.
