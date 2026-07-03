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
