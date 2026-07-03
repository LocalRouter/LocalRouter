# MCP 2026-07-28 Implementation Plan (Backward Compatible)

Implements the gap analysis in `plan/2026-07-02-MCP_2026_07_28_GAP_ANALYSIS.md`.

**Hard requirement:** full backward compatibility. Every commit keeps
2025-11-25 (and older HTTP+SSE) clients and backends working. New-revision
behavior is selected per peer:

- **Downstream (gateway as server):** revision detected per request —
  `MCP-Protocol-Version` header, `_meta` `io.modelcontextprotocol/protocolVersion`,
  or legacy `initialize` handshake ⇒ 2025-11-25 path.
- **Upstream (bridge as client):** per-backend revision probed once
  (`server/discover`, falling back to `initialize`) and cached.

## Progress tracking

Tracked in the session task list (tasks below) and by per-phase commits.

**Status 2026-07-03:** Phases A and B fully landed, plus C1.
- A1/A2 (`feat(oauth)`): iss validation + issuer-bound credentials
- B1 (`feat(mcp)` protocol types), B2+B3 (stateless downstream lifecycle
  + HTTP headers), B4 (extensions passthrough), B5 (backend discover
  probe + upstream stateless), B6 (per-revision method sets/error codes)
- C1 (`feat(server)`): subscriptions/listen stream
- **C2 (MRTR) landed 2026-07-03** for elicitation in both directions
  (`feat(mcp)` MRTR commit). Scope decision: MRTR covers *elicitation*
  input requests only — sampling is Deprecated in 2026-07-28 (its
  replacement is direct LLM API integration, i.e. LocalRouter itself),
  so no sampling input-request bridging was built; passthrough sampling
  toward stateless clients falls back to Direct/local-approval modes as
  before. Firewall Ask remains a local desktop-UI interaction (it is
  not client-directed input). Original design sketch: when a stateless
  downstream client's tools/call triggers a backend elicitation /
  passthrough-sampling / firewall Ask, the gateway cannot push over SSE.
  Instead: run the backend call as a task; when a server-initiated
  request fires, park the task + pending manager ids in an MrtrManager
  keyed by an opaque `requestState`, and resolve the original call with
  `resultType: "input_required"` + `inputRequests`. The client retries
  the call carrying `inputResponses` + `requestState`; the gateway
  submits the responses through the existing managers (elicitation
  schema validation already exists), awaits the parked task, and
  returns its result (or another input_required round). Upstream:
  handle `input_required` results from stateless backends by resolving
  through the same managers and retrying with `inputResponses`.
  Watch: timeout/cleanup of parked tasks; firewall Ask flows for
  stateless peers currently still block the HTTP request (works, but
  MRTR is the spec-preferred shape).

Known accepted tradeoffs:
- First session to a misbehaving backend that neither answers nor
  rejects `server/discover` pays the 5s probe timeout once (cached
  after).
- `broadcast_and_return_first` still sends ping/logging to stateless
  backends when a legacy client broadcasts; they answer method-not-found
  harmlessly.
- Per-request `_meta` logLevel is captured on the session but not yet
  translated into `logging/setLevel` for legacy backends.

## Phase A — Tier-0 security (spec-version independent)

- **A1. RFC 9207 `iss` validation (SEP-2468).** Add `issuer` to the AS
  metadata we fetch, thread the expected issuer into pending browser flows,
  and validate the `iss` query param on authorization callbacks when present
  (MUST reject mismatch). Files: `crates/lr-oauth/src/browser/*`,
  `crates/lr-mcp/src/oauth.rs`, `oauth_browser.rs`.
- **A2. Issuer-bound credentials (SEP-2352).** Persist the issuer with cached
  tokens/secrets (`{server_id}_issuer` keyring entry). On use/refresh, if the
  current AS issuer differs from the recorded one: drop cached credentials and
  force re-auth. Legacy entries without a recorded issuer adopt the current
  issuer on first use (no user-visible migration).

## Phase B — dual-revision protocol core

- **B1. Protocol types** (`crates/lr-mcp/src/protocol.rs`):
  `ProtocolRevision` enum (`V2025_11_25`, `V2026_07_28`) with parse/cmp;
  `_meta` key constants; new error codes (`HEADER_MISMATCH -32020`,
  `MISSING_REQUIRED_CLIENT_CAPABILITY -32021`, `UNSUPPORTED_PROTOCOL_VERSION
  -32022`); resource-not-found code selection by revision (`-32002` old /
  `-32602` new); `resultType` injection helper; `server/discover`
  request/response types; `CacheableResult` fields (`ttlMs`, `cacheScope`).
- **B2. Stateless downstream lifecycle** (`gateway.rs`, `routes/mcp.rs`):
  - `server/discover` handler advertising `["2026-07-28", "2025-11-25"]`,
    merged capabilities, and server identity.
  - Per-request `_meta` parsing (protocolVersion / clientInfo /
    clientCapabilities) stored on the request context.
  - Lazy session materialization: `get_or_create_session` (keyed by
    authenticated `client_id`) creates per-client backend transports on first
    use so requests work without a prior `initialize`. The legacy
    `initialize` path keeps its current behavior.
  - Reject unsupported versions with `UnsupportedProtocolVersionError`.
- **B3. Downstream HTTP transport hardening** (`routes/mcp.rs`):
  read `MCP-Protocol-Version`; validate `Mcp-Method`/`Mcp-Name` against the
  body for new-revision requests (mismatch ⇒ `-32020`); tag responses with
  `resultType` and cache fields when the peer speaks 2026-07-28.
- **B4. Merger updates** (`gateway/merger.rs`): deterministic tool ordering;
  `ttlMs`/`cacheScope: "private"` on list results (new revision only);
  pass through `extensions` capability maps; schema-stripping tolerant of
  `$ref`/`oneOf` roots without `properties`.
- **B5. Upstream client updates** (`transport/sse.rs` + bridge):
  send `MCP-Protocol-Version` and `Mcp-Method`/`Mcp-Name` headers on POSTs;
  probe backends with `server/discover` → fallback to `initialize`; record
  per-backend revision; treat results without `resultType` as `"complete"`;
  re-issue broken in-flight streamable-HTTP requests (no Last-Event-ID).
- **B6. Method-set differences per revision:** `ping`, `logging/setLevel`,
  `notifications/roots/list_changed` continue for old peers; for new peers,
  per-request `_meta` `logLevel` replaces `setLevel`. Error-code mapping at
  the gateway boundary when bridging revisions.

## Phase C — architectural pieces

- **C1. `subscriptions/listen`** downstream: long-lived POST response stream
  with typed opt-ins and `subscriptionId` tagging; wire the existing
  list-changed forwarding into it. Legacy GET SSE stays for old clients.
- **C2. MRTR (SEP-2322):**
  - Downstream (new clients): elicitation / sampling-passthrough / firewall
    approvals return `InputRequiredResult` with `requestState`; retried
    requests with `inputResponses` resume via the existing managers.
  - Upstream (new backends): handle `resultType: "input_required"` from
    tool calls by resolving through the existing approval/elicitation flows
    and retrying with `inputResponses`.
  - Old⇄new bridging in both directions.

## Non-goals (this round)

- Tasks extension, MCP Apps, trace-context propagation (tracked in the gap
  analysis as Tier 2).
- Removing any deprecated feature (sampling/roots/logging stay).
- Client ID Metadata Documents (follow-up; we have no DCR today).

## Mandatory Final Steps

1. **Plan Review** — re-check each item above against the implementation.
2. **Test Coverage Review** — new/modified paths covered, including
   dual-revision negotiation matrices.
3. **Bug Hunt** — fresh-eyes pass over the diffs.
4. **Commit** — per-phase conventional commits; no push unless asked.
