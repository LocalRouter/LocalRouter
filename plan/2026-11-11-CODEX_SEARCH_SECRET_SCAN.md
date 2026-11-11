- [x] Verify the upstream standalone-search schema and existing proxy/scanner boundaries.
- [x] Recognize Codex alpha/search and reuse shared masking/restoration without rejecting malformed, unknown, or unavailable-storage requests.
- [x] Scan conversation input and plaintext search commands; preserve non-secret protocol metadata and opaque encrypted fields.
- [x] Record search requests/responses in Monitor with accurate API attribution and searchable previews.
- [x] Add schema, escaping, fragmentation, isolation, opaque-data, fail-open and real HTTP proxy regressions.
- [x] Plan review: check every requirement against implementation.
- [x] Test coverage review: cover new paths, errors and edge cases.
- [x] Bug hunt: review protocol compatibility, byte preservation and routing/policy interactions.
- [x] Run required stable Rust/workspace checks and applicable frontend checks.
- [ ] Commit signed changes, merge to current master with merge commits, and push without overwriting concurrent work.

User requested implementation and merge to master. No release dispatch is included in this follow-up. Do not access the production credential store or restart production. Use mock/file credential storage and controlled synthetic traffic for tests. All internal Secret Scan failures must keep forwarding requests and responses. Preserve provider-owned encrypted_output/encrypted_content and signatures byte-for-byte. Do not treat this search endpoint as a Responses resource or restore opaque search continuation tokens. Use current master in the isolated existing worktree and preserve other checkouts.


Implementation review: the narrow alpha/search route now uses shared AppFirewall masking/restoration and Search API attribution. Search commands are extracted with original decoded string values, including queries, find patterns, URLs and future command entries; optional conversation input uses the existing Responses extractor. Provider encrypted_output and typed encrypted_content stay opaque in parsed and rolling JSON paths. Search commands/answers are displayed in Monitor rather than hidden or mislabeled as ranked-only retrieval.

Coverage so far: real HTTP proxy calls with queries only and with context, masked upstream bytes and Monitor captures, fragmented Unicode/JSON responses, preserved encrypted state and unknown fields, memory-only storage outage, malformed upstream content, client isolation, and first-chunk delivery before the provider body ends. Production AppFirewall tests also cover missing scanner/invalid bodies without Secret Scan rejection. Frontend tests/build and website build have passed.

The installed Codex 0.166 regression forces tools.web__run against a local TLS provider using synthetic auth and mock credential storage; the proxy rejects public destinations. The first run reached alpha/search, sent a masked query, consumed its result and completed exact restored tool/file/reply checks, but an extra assertion incorrectly expected encrypted state replay. Pinned upstream codex-rs/ext/web-search/src/tool.rs uses response.output/response.results and does not replay encrypted_output; the invalid replay assertion was removed. Opaque field preservation remains asserted by direct JSON/fragmented HTTP tests. Sources: https://github.com/openai/codex/blob/rust-v0.166.0/codex-rs/ext/web-search/src/tool.rs and codex-rs/codex-api/src/search.rs.

Final review completed: required command/input extraction, nonblocking error paths, opaque data preservation and Monitor presentation are implemented and covered. Stable Rust validation passed 5,143 tests with zero failures (86 ignored); the explicit installed Codex standalone-search test also passed with search, tool execution, exact file output and fresh-process resume. All-target Clippy and formatting passed. Frontend validation passed 151 tests, desktop build and website build/check. Evidence: /tmp/localrouter-codex-search-evidence/results.json, frontend-results.json and installed-search.log. No native credential storage was accessed.
