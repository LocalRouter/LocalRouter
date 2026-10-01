# MCP, tools, context and local sessions review — 2026-10-01

## Progress and approach

- [x] Read the project guide; record the initial inventory and preserve unrelated edits.
- [x] Structurally inspect every assigned first-party Rust module and crate manifest.
- [x] Deep-review trust boundaries, session lifecycle, concurrency and resource handling.
- [x] Implement concrete improvements with focused local regressions.
- [x] Review implementation against findings and test coverage; perform a fresh bug hunt.
- [x] Coordinate CPU-only offline validation with the root agent and record exact initial outcomes; final stable reruns belong to the consolidated report.
- [x] Complete the detailed findings, coverage and remaining limitations below.

No commits are made during this shared review, as instructed by the coordinating agent. No GPU execution, model inference/download, external provider calls, credential access or application launch is permitted.

## Inventory at the start of review

| Crate | Rust files | Rust lines (including tests) |
|---|---:|---:|
| lr-mcp | 35 | 29,707 |
| lr-mcp-via-llm | 8 | 8,357 |
| lr-context | 8 | 5,442 |
| lr-memory | 5 | 2,918 |
| lr-skills | 8 | 2,520 |
| lr-marketplace | 7 | 3,542 |
| lr-coding-agents | 6 | 3,844 |
| lr-proxy | 21 | 7,551 |
| lr-responses-sessions | 1 | 369 |

The inventory totals 99 Rust files and 64,250 lines. Structural coverage and deep inspection are distinct; this review does not claim a line-by-line proof of correctness across that volume.

## Findings and changes

Implemented changes and per-section findings are detailed below.

## Validation

Initial compilation and 800 CPU/local tests passed. The coordinator subsequently ran the final shared-boundary suites on stable Rust 1.99.0: all 297 tests passed across six crates; see the final validation note below.

## Implemented corrections (source review complete; validation tracked separately)

### 1. Response-history isolation between authenticated clients

`lr-responses-sessions/src/lib.rs` now exposes `get_active_for_client(id, client_id, retention)`. The SQL query filters ownership before decoding the row. A foreign response ID and a missing response ID both return `None`; lookups do not refresh last activity. Existing unrestricted `get_active` remains for administrative/internal uses. A new regression covers own, foreign and missing response IDs and verifies the original timestamp stays unchanged.

The review found the actual leak in `lr-server/src/routes/responses.rs`: a supplied `previous_response_id` was loaded without comparing its owner with the authenticated client. The API agent migrated that caller to the owner-scoped method. This is a coordinated fix spanning the assigned store and another agent's server code; the server agent owns its endpoint behavior and endpoint tests.

### 2. Concurrent memory session creation

`lr-memory/src/session_manager.rs` uses a DashMap entry lock through lookup and replacement. Previously two requests could both see no session, generate separate transcript paths, and overwrite the map entry. The new regression synchronizes 16 threads for each of 16 independent clients and verifies that exactly one request creates the shared session.

### 3. Memory expiry racing with renewed activity

The same manager now uses conditional removal under the DashMap shard lock. The previous sequence removed the session, checked its age, and reinserted it if refreshed. That exposed a gap in which another request could create a new session that was subsequently overwritten. Existing expiry/max-duration/touch tests exercise ordinary behavior; the lock-level reasoning covers the previously exposed removal gap.

### 4. Transcript initialization no longer truncates concurrent data

`lr-memory/src/transcript.rs` now creates/opens the transcript in append mode. An already-started request can append an exchange before another request's initialization finishes; `fs::write(path, "")` previously erased that exchange. A regression writes an exchange first, initializes afterward, and verifies byte-for-byte preservation.

Memory short display IDs and expiry logging also use character-safe shortening instead of slicing an arbitrary UTF-8 string at byte 8. A Unicode regression covers both timestamp-prefixed and legacy-style names.

### 5. Skill ReadFile confines resolved symlinks

`lr-skills/src/mcp_tools.rs` routes reads through `SkillManager::get_resource`, reusing its canonical path containment check. Discovery includes symlinked files, so membership in the discovered file list did not prove containment. The Unix regression creates a permitted reference and a symlink to a sibling private file: the permitted read succeeds and the symlink escape fails.

This does not claim protection against a hostile local process swapping filesystem entries between validation and open; fully race-resistant traversal would require directory-relative no-follow operations.

### 6. Marketplace downloads validate destinations and payloads

`lr-marketplace/src/skill_sources.rs` validates portable relative manifest paths before download, rejecting absolute paths, parent traversal, Windows drive/alternate-stream syntax, backslashes and NULs. Additional files cannot replace the authoritative `SKILL.md`. It checks existing path components for symlinks before creating directories or writing. The implementation rejects unsuccessful HTTP responses rather than saving an HTML/error response as an installed skill.

Downloads are streamed into a bounded buffer: at most 16 MiB per file and 64 MiB per installation. Limits apply to actual received chunks as well as advertised Content-Length, so omitted/misleading length headers do not disable the limit. Four synthetic loopback HTTP responses test unsuccessful status, excessive Content-Length, an excessive chunked body with no Content-Length, and a successful file that consumes the aggregate budget.

`lr-marketplace/src/lib.rs` now exposes `MarketplaceService::download_skill(&listing) -> Result<PathBuf, MarketplaceError>`, including validation that `source_label` and `name` are each a single directory component. Other agents migrated the real Tauri direct-install path and launcher callback to this API, removing duplicated unsafe download code rather than leaving this correction unused.

Additional regressions cover portable path rules, nested directory creation, symlinked parent directories, symlinked manifest files and preservation of outside content. A late regression verifies reserved manifest names, including normalized `SKILL.md/` and `SKILL.md/.`, and traversal paths are rejected before filesystem/network work in the download client. Installation is still not transactional: a later download failure can leave a partial directory; see follow-ups.

### 7. Script output stays bounded and async timeouts actually terminate

`lr-skills/src/executor.rs` drains stdout/stderr while retaining only the latest 1 MiB per stream. Both sync and async execution use the bounded reader, so a noisy script cannot grow an unbounded Vec for the entire timeout. Pipes continue draining after the capture cap, avoiding an artificial child-process deadlock.

The async path previously awaited stdout/stderr EOF before killing a timed-out child, so the child could keep those pipes open indefinitely and defeat its timeout. It now kills first, then allows at most one second for pipe draining, aborting unfinished drain tasks if descendants keep descriptors open. The background-owned child uses kill-on-drop for cancellation/runtime shutdown. Captured byte tails are decoded lossily so a cut through a Unicode character does not erase all output.

Regressions verify the bounded capture retains the suffix and run only a synthetic local `/bin/sh` script containing `exec sleep 10` with a zero-second timeout; completion must be reported promptly. No coding agent, model, package manager or GPU program is executed.

### 8. Context truncation honors its byte limit

`lr-context/src/truncate.rs` no longer unconditionally includes the first/last line when either line alone exceeds its budget. That previously allowed a huge leading/trailing line to defeat a supposedly bounded tool response. It falls back to character-safe truncation whenever line-aligned edges cannot fit. The regression covers oversized first and last lines independently and together, Unicode content, and budgets from zero through 1000 bytes.

### 9. Context reads avoid overflow and quadratic long-line indexing

`lr-context/src/lib.rs` uses saturating addition for offset plus requested limit. A read beginning at line 2 with `usize::MAX` previously overflowed in a debug build (or wrapped in an optimized build). A regression now verifies the correct remaining lines.

Long-line subdivision computes UTF-8 byte offsets once instead of repeatedly summing all prior character widths for every slice. This changes that indexing step from quadratic growth to a linear pass while retaining existing numbered sub-line behavior and UTF-8-safe boundaries. Existing long-line, sub-offset, Unicode and large-document tests cover the behavior.

### 10. MCP WebSocket authentication headers reach the handshake

`lr-mcp/src/transport/websocket.rs` constructs the handshake request and copies the configured headers into it with validated header names/values. Previously headers were stored in the struct but `connect_async(&url)` never sent them, breaking authenticated WebSocket MCP servers. A loopback server verifies a synthetic `x-test-auth` header is received.

### 11. Concurrent/cancelled MCP WebSocket writes retain the connection

The WebSocket sink is now held in a Tokio mutex and borrowed while awaiting a send. Previously each sender took the sink out of an Option, causing overlapping requests to report “write handle not available”; cancellation during the await could permanently drop the only write handle. The local regression cancels a request waiting on the write lock, then successfully submits 16 concurrent JSON-RPC requests with 256 KiB synthetic payloads and checks correlation of restored IDs.

### 12. Cancelled MCP requests clean up pending state

`lr-mcp/src/transport/mod.rs` defines a scoped pending-request guard used by stdio, SSE and WebSocket sends. Dropping a caller's future now removes its response sender even if cancellation occurs during serialization/write/HTTP send/response wait. Existing explicit error cleanup remains harmless and idempotent. The WebSocket cancellation regression checks the pending map returns to empty.

### 13. MCP broadcast preserves backend errors

`lr-mcp/src/gateway/router.rs` now converts a JSON-RPC error response into a broadcast failure rather than treating it as a successful null result. A mock transport regression verifies an unsupported-method response is present in failures and absent from successes. Retry backoff also clamps the shift before arithmetic, preventing overflow for high configured retry counts while preserving the 10-second cap.

### 14. MCP transport logs do not print configured header values

SSE diagnostic messages log header names rather than entire request/response header maps. In particular this removes configured Authorization/custom-token values from ordinary debug logging. Upstream error bodies and URLs still have broader diagnostic privacy considerations; the change is intentionally specific to the confirmed header disclosure.

### 15. Mixed tool execution is claimed atomically and stays cancellable

`lr-mcp-via-llm/src/manager.rs` matches client tool-call results and removes the matching pending execution under one DashMap entry lock. It can no longer match one execution then remove a replacement inserted before its separate removal. A concurrent-consumer regression verifies a pending execution is claimed once.

`lr-mcp-via-llm/src/orchestrator.rs` awaits background tool handles while they remain owned by PendingMixedExecution. Previously taking the handles out before awaiting disabled the Drop implementation's ability to abort unfinished tools if the resume request was cancelled. Existing pending-execution Drop tests cover abort ownership; full mocked orchestrator integration tests provide CPU-only behavior coverage.

### 16. Coding-agent session status and listing correctness

`lr-coding-agents/src/manager.rs` subscribes to completion notifications before checking current state, closing a missed-wakeup window that could delay a completed session until the full wait timeout. It releases DashMap guards before asynchronously waiting for per-session locks during interruption. Session lists now sort the complete matching set before truncation; the previous arbitrary DashMap iteration limit could omit the newest sessions. Limit zero returns no sessions.

Two process-free regressions cover completion notification and newest-first limiting, including zero limit, wrong client and agent-type filtering. Real coding agents and their permission flows were not launched.

### 17. Proxy CA key permissions are restrictive before bytes are written

`lr-proxy/src/cert.rs` opens new secret files with Unix mode 0600 and tightens an existing open file's mode before writing bytes. Previously it wrote the key with default permissions and only afterward changed permissions. The existing root-key permission test and a new pre-existing-mode-0644 synthetic fixture cover the final modes/content. No real CA key or credential file was read during review.

### 18. Untrusted request trace headers cannot disable enforcement/accounting

This coordinated second-wave fix is implemented centrally in `lr-types/src/trace.rs` and documented in `foundations.md`. `RequestTrace::outbound_for` preserves the wire trace ID for correlation but resets its hop to 1. A public `X-LocalRouter-Trace: forged;hop=99` no longer grants “already checked” privileges. Proxy boundary tests verify the forged header still reaches the firewall and increments metrics; reverse proxy integration expectations retain correlation but reject duplicate privileges. The API agent owns equivalent server ingress tests.

Compatibility change: unauthenticated wire-based multi-hop deduplication is disabled; real multi-hop routes can now count/prompt more than once. Restoring that optimization requires authenticated, request-bound provenance. Internal deliberately trusted traces retain their existing behavior.

## Review coverage and retained behavior

This was a risk-driven review of all nine crate inventories, manifests, public interfaces, stateful operations, I/O boundaries and relevant test entry points. Deep review concentrated on the paths above. The following untouched areas were reviewed structurally and sampled semantically; “no additional fix” is not a correctness guarantee.

| Section | Inspected contracts and behavior | Additional result |
|---|---|---|
| MCP protocol and merger | JSON-RPC request/notification/response types, error constructors, capability/version types, namespace and catalog merge interfaces | No additional verified issue isolated in the sampled pure merge/protocol paths; no live protocol-conformance claim |
| Gateway access control | Permission hierarchy, Allow/Ask/Off resolution, session approval/denial precedence | Existing explicit denial/permission branching retained |
| Gateway tools/resources/prompts | Namespaced mappings, resource URI fallback, per-session transport extraction, virtual server dispatch and permission entry points | Follow-up on concurrent session refresh and live permission changes remains warranted |
| Approval managers | Pending-map lifecycle, timeout/response/cancellation handling, elicitation schema validation | Existing explicit timeout cleanup present; future-drop cleanup is less consistent than transport cleanup |
| Virtual servers | Skills, memory, marketplace, coding-agent and context tool ownership/configuration interfaces | Verified dependency fixes flow through shared implementations; no external tool execution performed |
| MCP manager/bridge/OAuth | Transport lifecycle surfaces, stdio cwd validation, stream relay signatures, test keychain injection and ignored package-download tests | Live process fleets, OAuth browser consent/redirects and upstream integration intentionally untested |
| MCP via LLM | Session hash matching, explicit key fast path, hidden-history reconstruction, guardrail gates, mixed/client tool dispatch, background-handle ownership | Mock provider/MCP integration only; heuristic session matching policy retained |
| Context | FTS query parameterization/sanitization, transactional replacement, search fallback, reads, chunking, truncation, hybrid rank fusion | No embedding model or vector benchmark run; concurrency of optional vector reindexing merits separate work |
| Memory | Client stores, transcript capture, session grouping, archives/compaction callback, search/reindex interfaces | Compaction uses mock callbacks in tests; no real summarization call |
| Skills | Discovery/frontmatter, archive entry confinement, manager snapshots, filesystem watcher, file reads, script executor | Archive expansion limits and watcher backpressure remain follow-ups |
| Marketplace | Registry requests, source cache, listing transforms, install config, popup approvals, callback boundaries, downloads | Existing registry status checks retained; download/install trust boundary improved as above |
| Coding agents | Ownership checks, start/say/resume/status/list/end lifecycle, discovery interfaces, command argument generation, approval service, bounded output ring | Agent binaries, account state and live work directories not exercised |
| Proxy | Authentication entry, host policy, CONNECT/plain HTTP/TLS/reverse interfaces, bounded captures, HTTP/WebSocket framing, wire metadata, active/passive interceptors | No real upstreams, root CA installation, system trust mutation, app listener or user traffic |
| Responses sessions | SQLite schema, persistence, active-window filtering, retention sweep, serialization, owner lookup | CPU in-memory tests; extreme retention values and storage quotas remain follow-ups |

## Remaining limitations and specific follow-ups

- **Coverage:** structural review covered the listed Rust modules; this is not a line-by-line proof of 64k lines. Generated assets, embedded models and third-party dependencies were not audited as first-party source.
- **Skill ZIPs:** enclosed entry names block lexical traversal, but extraction lacks an explicit total expanded-byte/file-count budget and an atomic completion marker. Malformed/huge archive fixtures should precede a broader extractor redesign.
- **Filesystem race resistance:** canonical/no-symlink checks prevent ordinary supplied-path escapes, but do not eliminate races against a hostile local process replacing directories during async I/O. Directory-relative handles/no-follow writes are the stronger boundary.
- **Transactional installs:** a network/error/limit failure can leave a partial installation directory. A staging directory plus atomic promotion would also improve recovery and cancellation behavior.
- **Script lifecycle:** output is now bounded and timeouts effective, but process-group termination/descendant guarantees remain platform-specific; asynchronous output files still update on completed capture rather than live streaming. PID-based temp paths and cleanup deserve a separate lifecycle design.
- **Long-lived maps:** per-client orchestration histories, seen-tool sets and approval stores need a unified memory quota/eviction policy; expiry alone is not a strict live-allocation cap.
- **MCP transport bounds:** line/event buffers and response bodies should receive protocol-appropriate frame limits. SSE multiline data and CRLF framing deserve dedicated interoperability fixtures. No sweeping protocol rewrite was attempted in this pass.
- **Gateway shutdown/reconnect:** SessionTransportSet snapshot/clear/close behavior, revision metadata cleanup and cancellation of approval futures warrant stress testing with real mocked connections and concurrent session resets.
- **Memory archival:** session rotation can leave expired transcript files for later maintenance; conversation detection versus forced close/compaction is not made fully transactional by the map-entry correction.
- **Catalog/embedding paths:** no GPU, model inference, downloads or benchmark execution was used. Optional vector indexing quality and consistency under concurrent reindexing remain outside runtime validation.
- **Coding-agent process policy:** concurrency reservation across simultaneous starts/resumes and authorization changes between requests need broader integration fixtures with a dedicated fake agent protocol before lifecycle changes.
- **Retention/storage:** persistent transcript/session stores lack comprehensive quotas; extreme signed retention settings can still merit input validation before arithmetic.
- **Trace deduplication:** correlation is preserved, but repeated multi-hop accounting/prompts are an intentional security tradeoff until provenance is authenticated.

## Validation evidence

Initial offline build command (no test execution during compilation):

```sh
RUSTC_WRAPPER= LOCALROUTER_SKIP_CATALOG_FETCH=1 \
CARGO_TARGET_DIR=/private/tmp/localrouter-review-target \
cargo test --offline -p lr-context -p lr-memory -p lr-skills \
  -p lr-marketplace -p lr-mcp -p lr-mcp-via-llm \
  -p lr-coding-agents -p lr-proxy -p lr-responses-sessions --lib --no-run
```

The ambient toolchain was Homebrew Rust 1.98.1; compilation succeeded in 11m25s. The root agent uses rustup stable 1.99.0 for final CI-parity checks. This toolchain difference was identified and communicated; no claim is made that the initial build itself used stable 1.99.0.

The compiled test binaries then ran directly in three parallel processes, four test threads each, with 180-second process limits. Escalation was needed because the sandbox blocks loopback socket binds. Only reviewed synthetic/local fixtures ran: no GPU work, model downloads, inference, live provider calls or application launch. The three ignored MCP tests require package managers/downloads and stayed ignored.

| Crate | Passed | Failed | Ignored | Initial binary coverage |
|---|---:|---:|---:|---|
| lr-context | 140 | 0 | 0 | Latest code |
| lr-memory | 44 | 0 | 0 | Latest code |
| lr-skills | 34 | 0 | 0 | Latest code, including bounded output and async timeout regressions |
| lr-marketplace | 28 | 0 | 0 | Main download changes; later normalized reserved-name hardening needs rerun |
| lr-mcp | 318 | 0 | 3 | Latest code, including authenticated/concurrent/cancelled loopback WebSocket fixture |
| lr-mcp-via-llm | 101 | 0 | 0 | Latest code and mocked orchestrator tests |
| lr-coding-agents | 58 | 0 | 0 | Latest code, process-free manager tests |
| lr-proxy | 72 | 0 | 0 | CA key fix; late trace boundary/tests require rebuilt binary |
| lr-responses-sessions | 5 | 0 | 0 | Latest owner-isolation API/test |
| **Total** | **800** | **0** | **3** | Initial direct-binary pass |

Logs: `/private/tmp/localrouter-review-mcp-tests/*.log`. Late trace/types, marketplace and foundational corrections subsequently passed the coordinator's final stable rerun; see the final validation note. `rustup run stable rustfmt --edition 2021 --check` passed for all then-modified Rust files. Final normalization changes were formatted with stable rustfmt afterward.

## Full primary Rust file inventory

Every path below was included in the structural inventory/search pass. “Changed / focused review” identifies implemented paths and their close regression files. “Structural / sampled” explicitly avoids claiming line-by-line deep review of an entire large module. Tests/benchmarks listed as structural were inspected for their execution requirements; vector benchmarks were not run.

| File | Review depth |
|---|---|
| `crates/lr-mcp/src/bridge/mod.rs` | Structural / sampled |
| `crates/lr-mcp/src/bridge/stdio_bridge.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/access_control.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/coding_agent_approval.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/context_mode.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/elicitation.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/firewall.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/gateway.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/gateway_prompts.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/gateway_resources.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/merger.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/mod.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/router.rs` | Changed / focused review |
| `crates/lr-mcp/src/gateway/sampling.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/sampling_approval.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/session.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/streaming_notifications.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/tests.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/types.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/virtual_coding_agents.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/virtual_marketplace.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/virtual_memory.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/virtual_server.rs` | Structural / sampled |
| `crates/lr-mcp/src/gateway/virtual_skills.rs` | Structural / sampled |
| `crates/lr-mcp/src/lib.rs` | Structural / sampled |
| `crates/lr-mcp/src/manager.rs` | Structural / sampled |
| `crates/lr-mcp/src/oauth.rs` | Structural / sampled |
| `crates/lr-mcp/src/oauth_browser.rs` | Structural / sampled |
| `crates/lr-mcp/src/protocol.rs` | Structural / sampled |
| `crates/lr-mcp/src/transport/mod.rs` | Changed / focused review |
| `crates/lr-mcp/src/transport/session_transport_set.rs` | Structural / sampled |
| `crates/lr-mcp/src/transport/sse.rs` | Changed / focused review |
| `crates/lr-mcp/src/transport/stdio.rs` | Changed / focused review |
| `crates/lr-mcp/src/transport/websocket.rs` | Changed / focused review |
| `crates/lr-mcp-via-llm/src/gateway_client.rs` | Structural / sampled |
| `crates/lr-mcp-via-llm/src/integration_tests.rs` | Structural / sampled |
| `crates/lr-mcp-via-llm/src/lib.rs` | Structural / sampled |
| `crates/lr-mcp-via-llm/src/manager.rs` | Changed / focused review |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | Changed / focused review |
| `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` | Structural / sampled |
| `crates/lr-mcp-via-llm/src/session.rs` | Structural / sampled |
| `crates/lr-mcp-via-llm/src/tests.rs` | Changed / focused review |
| `crates/lr-context/benches/vector_search.rs` | Structural / sampled |
| `crates/lr-context/src/chunk.rs` | Structural / sampled |
| `crates/lr-context/src/fuzzy.rs` | Structural / sampled |
| `crates/lr-context/src/hybrid.rs` | Structural / sampled |
| `crates/lr-context/src/lib.rs` | Changed / focused review |
| `crates/lr-context/src/search.rs` | Structural / sampled |
| `crates/lr-context/src/truncate.rs` | Changed / focused review |
| `crates/lr-context/src/types.rs` | Structural / sampled |
| `crates/lr-memory/src/compaction.rs` | Structural / sampled |
| `crates/lr-memory/src/lib.rs` | Structural / sampled |
| `crates/lr-memory/src/session_manager.rs` | Changed / focused review |
| `crates/lr-memory/src/tests.rs` | Changed / focused review |
| `crates/lr-memory/src/transcript.rs` | Changed / focused review |
| `crates/lr-skills/src/discovery.rs` | Structural / sampled |
| `crates/lr-skills/src/executor.rs` | Changed / focused review |
| `crates/lr-skills/src/fuzzy.rs` | Structural / sampled |
| `crates/lr-skills/src/lib.rs` | Structural / sampled |
| `crates/lr-skills/src/manager.rs` | Structural / sampled |
| `crates/lr-skills/src/mcp_tools.rs` | Changed / focused review |
| `crates/lr-skills/src/types.rs` | Structural / sampled |
| `crates/lr-skills/src/watcher.rs` | Structural / sampled |
| `crates/lr-marketplace/src/install.rs` | Structural / sampled |
| `crates/lr-marketplace/src/install_popup.rs` | Structural / sampled |
| `crates/lr-marketplace/src/lib.rs` | Changed / focused review |
| `crates/lr-marketplace/src/registry.rs` | Structural / sampled |
| `crates/lr-marketplace/src/skill_sources.rs` | Changed / focused review |
| `crates/lr-marketplace/src/tools.rs` | Structural / sampled |
| `crates/lr-marketplace/src/types.rs` | Structural / sampled |
| `crates/lr-coding-agents/src/approval.rs` | Structural / sampled |
| `crates/lr-coding-agents/src/discovery.rs` | Structural / sampled |
| `crates/lr-coding-agents/src/lib.rs` | Structural / sampled |
| `crates/lr-coding-agents/src/manager.rs` | Changed / focused review |
| `crates/lr-coding-agents/src/mcp_tools.rs` | Structural / sampled |
| `crates/lr-coding-agents/src/types.rs` | Structural / sampled |
| `crates/lr-proxy/src/active.rs` | Changed / focused review |
| `crates/lr-proxy/src/anthropic.rs` | Structural / sampled |
| `crates/lr-proxy/src/cert.rs` | Changed / focused review |
| `crates/lr-proxy/src/error.rs` | Structural / sampled |
| `crates/lr-proxy/src/interceptor.rs` | Structural / sampled |
| `crates/lr-proxy/src/lib.rs` | Changed / focused review |
| `crates/lr-proxy/src/manager.rs` | Structural / sampled |
| `crates/lr-proxy/src/ollama.rs` | Structural / sampled |
| `crates/lr-proxy/src/openai.rs` | Structural / sampled |
| `crates/lr-proxy/src/passive.rs` | Changed / focused review |
| `crates/lr-proxy/src/resolver.rs` | Structural / sampled |
| `crates/lr-proxy/src/reverse.rs` | Structural / sampled |
| `crates/lr-proxy/src/systemone.rs` | Structural / sampled |
| `crates/lr-proxy/src/tap.rs` | Structural / sampled |
| `crates/lr-proxy/src/tls.rs` | Structural / sampled |
| `crates/lr-proxy/src/transport.rs` | Structural / sampled |
| `crates/lr-proxy/src/websocket.rs` | Structural / sampled |
| `crates/lr-proxy/src/wire.rs` | Structural / sampled |
| `crates/lr-proxy/tests/mitm_e2e.rs` | Structural / sampled |
| `crates/lr-proxy/tests/passthrough_e2e.rs` | Structural / sampled |
| `crates/lr-proxy/tests/reverse_e2e.rs` | Changed / focused review |
| `crates/lr-responses-sessions/src/lib.rs` | Changed / focused review |

## Final coordinator validation

Stable Rust 1.99.0 final shared-library run passed **297 tests, zero failures**: lr-local-models 88, lr-marketplace 29, lr-monitor 33, lr-monitoring 45, lr-proxy 73 and lr-types 29. This includes the late marketplace normalization, trace, metrics query/index and monitor callback changes. Both the model-library lock refinement and indexed metrics source predate their rebuilt test binaries. A subsequent marketplace-only rerun after lint-only fixture corrections passed all 29 tests. Log: `/private/tmp/localrouter-review-final-shared-tests.log`; rerun: `/private/tmp/localrouter-review-marketplace-final-tests.log`. Whole-workspace all-target Clippy results and the reverse-proxy integration run are recorded in the consolidated report.

Clippy prompted a meaningful nonempty-read assertion and direct struct initialization in the marketplace HTTP fixture. The WebSocket fixture has a narrowly scoped large-error allowance because the third-party handshake callback requires an HTTP response error type; production error handling is unchanged.

The stable reverse-proxy TCP integration suite also passed all **6 tests**, including forged-hop reset, correlation, forwarding, streaming and unreachable-upstream behavior. These use ephemeral loopback listeners and dummy upstreams. Workspace `cargo clippy --offline --workspace --all-targets -- -D warnings` passed after the fixture lint corrections.
