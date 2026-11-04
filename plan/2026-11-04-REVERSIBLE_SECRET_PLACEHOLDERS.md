- [x] Inspect existing scanning, redaction, session controls, tool execution and landing-page structure.
- [x] Specify placeholder format, isolation, lifecycle, response transformation and failure behavior.
- [x] Specify a landing-page section showing the complete secret/placeholder round trip.
- [x] Save and review this design; validate the documentation diff; commit and push only this plan.
- [ ] Implement the scoped in-memory vault and reversible text/JSON transforms.
- [ ] Wire request protection and response restoration into supported gateway codecs.
- [ ] Protect MCP-via-LLM iterations and restore arguments before tool approval/execution.
- [ ] Add explicit configuration, approval UI, types and demo support.
- [ ] Implement the landing-page section and update secret-scanning documentation.
- [ ] Plan Review: reconcile implementation against every requirement and supported-path claim below.
- [ ] Test Coverage Review: cover lifecycle, isolation, streaming, serialization, tools and failures.
- [ ] Bug Hunt: review secret exposure, approvals, retries, caches, concurrency and protocol finalization.
- [ ] Run appropriate stable Rust, frontend, website and focused integration checks.
- [ ] Commit and push validated implementation, preserving unrelated changes and including automatic catalog updates.

# Reversible secret placeholders

Status: design only; no feature code or website changes are implemented by this task.

## Goal and example

Keep selected secrets out of model-visible requests while allowing an assistant to reuse them verbatim in its answer or a tool call. LocalRouter retains the original value locally, sends an opaque but recognizable placeholder, then restores an exact echoed placeholder at the trusted destination.

Example (synthetic secret only):

1. Client sends `Write API_KEY=demo-only-secret-value into .env`.
2. LocalRouter holds the value in memory and sends `Write API_KEY=[[LR_SECRET_v1_API_KEY_<scope>_<entry>]] into .env`.
3. Model produces `write_file({"path":".env","content":"API_KEY=[[LR_SECRET_v1_API_KEY_<scope>_<entry>]]"})`.
4. LocalRouter restores the parsed argument to `API_KEY=demo-only-secret-value` before the existing tool approval and execution path. The file contains the original value.
5. If a tool returns that value, LocalRouter masks it again before the next model call.

For client-executed tools, step 4 occurs while returning the tool call to the authenticated caller. LocalRouter cannot itself guarantee that caller's permissions, file behavior or logs. For LocalRouter-executed MCP tools, restoration occurs before the gateway's permission/approval/execution boundary.

## Existing implementation and insertion points

- `crates/lr-secret-scanner/src/redaction.rs`: existing `Redaction` replaces matches with the same irreversible `REDACTION_MESSAGE`; its Debug output exposes only a count. Keep that API and behavior intact.
- `crates/lr-server/src/routes/pipeline.rs`: `run_secret_scan_check` applies approved redactions before guardrails/compression and provider conversion; `TurnContext` currently carries no restoration context. Add an explicit context there rather than reconstructing it at response time.
- `crates/lr-server/src/documents.rs`: scan projections and decoded text attachment rewriting already exist. Reversible mode must transform the decoded content, then re-encode it.
- Gateway routes include Chat, Responses and Messages codecs, plus Completions and System One. Do not assume that sharing request scanning automatically supplies response restoration for every route.
- `crates/lr-mcp-via-llm/src/orchestrator.rs` and `orchestrator_stream.rs`: model/tool loops, background tool execution and tool-result ingestion need the same context on every iteration.
- `crates/lr-mcp/src/gateway/gateway_tools.rs`: shared MCP argument approval and dispatch boundary. Restore only with an explicitly propagated execution context; a standalone MCP request must not resolve handles through a client-wide search.
- `crates/lr-providers/src/session.rs` and `crates/lr-server/src/routes/gateway_json.rs`: existing explicit local session controls and session headers. Reuse these rather than inventing an unrelated conversation identifier.
- `crates/lr-server/src/routes/chat.rs` creates a fresh monitor `session_id` per request. This is not a durable conversation vault key. Do not use the orchestrator's hash-based conversation matching as authorization to access a vault.
- `crates/lr-responses-sessions/src/lib.rs`: Responses continuation stores histories and cached responses in SQLite. Its disk persistence requires deliberate handling; an in-memory vault alone does not make those histories safe.
- `src-tauri/src/launcher/proxy.rs` applies existing secret redaction to provider-native HTTPS/WebSocket requests. Proxy response transformations require separate work; do not advertise reversible support there until implemented and tested.
- `website/src/pages/Home.tsx` already has a Safety Layer section and Secret Scanning demo. Add the round-trip explanation beside this story, using existing site typography, colors and component conventions.

## 1. Separate reversible replacement from irreversible redaction

Keep existing Redact Once, remembered redaction digests, Ask/Notify/Off, allow and deny semantics unchanged. A digest cannot reconstruct a secret. Never reinterpret an existing irreversible-redaction decision as permission to restore it.

Add an explicit per-client opt-in protection mode, tentatively `secret_placeholders: off | restore`, default `off`. The approval popup gains a distinct “Replace with restorable placeholder” action with concise text explaining that the model can reuse the secret through the handle. The enabled policy can select automatic replacement for scanner findings; it must not silently allow a request that a deny decision blocks. Define and test precedence: deny first, remembered irreversible redaction next, reversible policy for eligible remaining findings, then existing ask/allow/notify behavior. Existing ignored values and system-message exclusions remain honored and documented; this mode protects selected/detected values, not every possible secret.

Use the existing scanner and custom detection configuration. Once a value is registered for the active scope, exact-match masking of that known value runs on all supported model-visible content, independently of whether the detector would recognize it again. Unknown secrets still depend on scanner coverage. Handle nested tool argument JSON as structured text, not just escaped outer JSON.

## 2. Placeholder contract

Concrete wire grammar:

`[[LR_SECRET_v1_API_KEY_9f2d8b715cd44c65aa73e0b8dc638260_b8c630278d00416080f87d83070659ab]]`

- `LR_SECRET_v1` identifies the reserved format.
- `API_KEY` is a small, fixed, non-sensitive kind enum, e.g. API_KEY, PASSWORD, PRIVATE_KEY or SECRET. No original prefix/suffix, username, host, file path or credential-derived digest appears in the token.
- The scope and entry components each contain 128 cryptographically random bits encoded as 32 lowercase hex characters. Check for collisions against the registry and existing input; regenerate rather than alias two values. Random identifiers do not derive from secret bytes.
- The same exact value gets the same handle within its scope; different values get distinct handles; different scopes get unrelated handles. Deduplication intentionally reveals within-scope equality.
- Match the entire token exactly, including delimiters and case. Never fuzzy-match, repair, trim, decode arbitrary encodings or infer handles from partial IDs.
- Replacement is one pass over original spans. Never rescan inserted secrets/placeholders. Resolve overlapping detected spans by merging to the exact original covered substring and registering that whole substring, preserving byte-for-byte restoration.
- Names are illustrative proposals; finalize the format once and version it before enabling it publicly.

Inject a short trusted instruction only when active placeholders exist:

“LocalRouter has replaced confidential values with opaque tokens of the form [[LR_SECRET_v1_KIND_SCOPE_ENTRY]]. Treat each token as the complete original value. When reusing a value in text or tool arguments, copy its token exactly, including brackets. Do not change, split, encode, hash, or guess it. LocalRouter restores exact tokens locally after your response.”

Include only handles available to that inference, if a manifest is needed; never include originals. Preserve the provider's supported system/developer-role semantics and existing instructions. Count the instruction in prompt sizing. This is guidance, not a guarantee that the model will echo correctly.

## 3. Vault ownership and lifecycle

Implement a reversible transform/vault alongside irreversible `Redaction` in `lr-secret-scanner`; keep ownership in the server and pass a narrow context to dependent components. Do not create another service or database.

- Keys are authenticated client identity plus an explicit conversation identity when present. A caller-supplied session label alone is insufficient; it is always namespaced by client identity.
- Without a reliable conversation identity, allocate a request-local scope. Full-history clients can work without sticky sessions: their returned plaintext history is re-scanned/replaced on each request. Do not imply compatibility for clients that retain only old placeholders without a retained scope.
- A request gets an explicit set of handles it may resolve: those included in its outbound model-visible context, plus deliberately carried conversation references. No global lookup by token and no access to another scope's secrets.
- Bind Responses continuation to the verified owning client and the same local scope. Follow existing continuity controls; fail explicitly if the mapping is gone. Test branch/retry/concurrent requests without cross-conversation reuse or races.
- Keep originals in memory only, wrapped to avoid Serialize and plaintext Debug/Display; use zeroizing storage where practical, with no claim of protection against process compromise, swap or every transient serialization copy.
- Pin context for active streams and tools. Release request-local scopes at completion/cancellation; release durable scopes on reset, expiry or client deletion. Use existing session lifecycle/retention where applicable, with bounded entry/byte budgets. Choose actual limits from existing configuration/policy during implementation; do not invent a silent eviction timeout.
- Exhaustion returns a local protection error before dispatch; never fall back to sending plaintext. Restart loses the map. Clearly explain that resumed placeholder-only conversations may need the original secret supplied again.
- Do not persist a mapping or original through settings, remembered digests, response stores, caches, monitor payloads or logs. Store tokenized histories/cached responses for protected turns. Re-read of a stored response needs its live scope and owner; after scope loss it fails explicitly rather than writing a stale token into a file.

## 4. Transform boundaries and correctness

Outbound: identify/authorize replacement, tokenize supported content, inject placeholder instructions, then run all provider-backed auxiliaries on the protected view. Verify compression, decision routing, guardrail providers, fallback/retry and each orchestrator iteration cannot see the original. Keep any local approval view separate and privacy-preserving. Exact known-value masking also covers model-visible tool results and replayed history.

Inbound: transform model text and executable tool-argument string values after decoding provider wire formats and before final client serialization or local tool execution. Restore substrings such as `Bearer <handle>` and multiline file contents. Parse JSON-encoded `function.arguments`, recursively restore its string values and serialize properly; never raw-replace inside serialized JSON. Quotes, backslashes, CR/LF and Unicode in secrets must stay valid JSON and restore the exact string value. Handle content blocks, legacy function-call forms where supported and parallel calls by their actual identities.

Do not transform protocol IDs, tool names, object keys, URLs in transport metadata, accounting fields, signatures, opaque reasoning state or binary data. If sensitive content is found in an unsupported field, block or use the existing explicit user decision; never mutate it blindly or silently claim protection. Binary/PDF/image content and base64/URL-encoded variants outside supported decoding are not covered automatically. A model-side tool/hosted sandbox executes before LocalRouter sees its arguments: reject that combination when secret restoration is required, or require an implemented local execution path.

Schemas that demand a credential's exact pattern/length/enum can reject a placeholder. Detect known incompatible strict schemas before dispatch and return a clear error; do not send plaintext or weaken the user's schema silently. Bound schema validation and document unsupported cases.

LocalRouter-executed tools: restore to an execution-only copy, then run all relevant schema validation, permission, firewall and destination checks against the actual effective arguments. Show masked values in normal approval UI, with explicit local reveal if already supported. Recheck after user edits. Never approve one argument set and execute another. Keep the assistant/history view tokenized. Apply the same rules to background calls and parallel tools.

Client-executed tools: restore before returning the tool call. The caller receives the original and owns further approvals/execution. Generic text responses also restore exact handles, as requested; do not restore unrelated response metadata.

Restoration does not authorize tool use. A model that can emit a known handle can ask a permitted tool to use that secret. Existing tool/destination approvals remain essential. Prompt injection can attempt this even though the model never receives the bytes; this feature prevents model-provider disclosure of selected secret values, not all exfiltration or unsafe tool behavior.

## 5. Streaming

Never replace independently in raw SSE/network chunks. Decode events first and transform logical text channels, keeping separate state per choice/content block/tool-call ID. A text transformer retains the longest suffix that could start an exact active token; a full match emits the original once. Unicode and SSE framing remain codec responsibilities.

For tool arguments in the first implementation, buffer the complete bounded argument JSON per call, validate/restore/serialize, then emit protocol-correct argument deltas and terminal snapshots containing the same restored value. This trades argument-stream latency for correctness; ordinary prose can continue streaming. Preserve tool identity/index ordering, stop reasons, usage, cancellation and errors. Reconcile terminal-only/missing argument suffixes through the existing completed-arguments handling before restoration. Never emit a partial tool call for execution.

Maintain both tokenized internal/monitor and restored caller views; finalize Responses/Messages terminal events and cached response reads consistently. Do not restore once in a delta and then duplicate the original from a terminal snapshot. Provider token usage describes the actual tokenized exchange, not the restored character length.

Unknown/wrong-scope/expired reserved handles in executable arguments cause an explicit local error before the call is returned or executed. A malformed reserved marker in executable arguments is also an error. In prose leave unknown or incomplete markers literal and expose a local diagnostic without a secret. Never guess a replacement. An interrupted tool-argument stream produces no executable call; a prose stream can only report errors using its protocol once headers have been sent.

## 6. Support rollout

1. Core vault/transforms and Chat Completions JSON plus streaming, including client-executed file-write calls. Request-local scopes first; integrate existing explicit sessions without heuristic matching.
2. MCP-via-LLM JSON/streaming, background tools and every repeated provider dispatch; protect tool results. Reversible mode must reject automatic MCP orchestration until this phase passes.
3. Responses and Messages JSON/streaming, stored-response retrieval, previous_response_id, caches and continuity; legacy Completions separately. Audit System One and other scanner consumers; leave irreversible behavior there until a meaningful supported restoration path is defined.
4. HTTPS/WebSocket proxy provider-native response adapters as a separate deliverable. Leave passive/opaque traffic unsupported. Gate unsupported endpoints locally when a client has requested reversible protection; do not silently downgrade to plaintext or imply reversible restoration.
5. Land configuration/UI/docs and the public landing section with the verified support scope. Do not publish feature claims before the corresponding backend behavior ships.

Ship the backend and user-facing settings together for the first supported gateway path; later phases expand the support matrix. Each protocol must pass the same test contract before it is enabled.

## 7. Landing-page section: “Keep the secret. Send the placeholder.”

Add a dedicated section adjacent to Safety Layer/Secret Scanning in `website/src/pages/Home.tsx`, backed by a small reusable `SecretPlaceholderDiagram.tsx`. Keep the existing detection/approval demo; the new section explains reversible reuse rather than duplicating that popup.

Suggested copy:

**Keep the secret. Send the placeholder.**

“LocalRouter holds the original value locally and sends a unique placeholder to the model. When the model copies it into a reply or tool call, LocalRouter puts the original value back.”

Supporting line: “A file-write call gets the real credential. The model sees the placeholder.”

Qualify the section with the shipped protocol scope and a link to `/docs/secret-scanning`; explain in those docs that selected/detected secrets are covered and tool permissions still apply. Do not claim that all secrets are caught, that tools never receive secrets, or that existing provider-key keychain storage is where the ephemeral vault lives.

Visual composition:

- Show Client / LocalRouter / Model as a left-to-right request path and a returning Model / LocalRouter / Tool path underneath. LocalRouter spans both rows, with a local “Held in memory” card connecting the replacement and restoration steps.
- Show a synthetic original `API_KEY=demo-only-secret-value` entering LocalRouter and staying in the local holding card. Only the colored placeholder chip crosses the clearly labeled model boundary.
- Return the exact same chip inside `write_file(.env, API_KEY=<placeholder>)`; at LocalRouter it becomes the synthetic original, and the final `.env` preview contains that original.
- Use one consistent accent for the handle, another for the held value. Label every state (“Original”, “Placeholder sent”, “Placeholder returned”, “Original restored”) so color alone carries no meaning.
- The full wire token is long: display an explicitly shortened token in the overview and reveal the identical full synthetic token in an expandable example. Clearly mark abbreviations as presentation only; the actual model must copy the complete token.
- Use inline SVG/HTML/CSS for crisp diagrams, local assets and selectable text. No AI bitmap is necessary. Use established project icons; no invented provider logos, emojis, external assets or tracking.
- Optional subtle arrow/chip motion illustrates request then return. Supply a static complete diagram with prefers-reduced-motion, keyboard-friendly controls if interactive, a useful accessible description and a stacked mobile layout. Do not depend on animation to explain the feature.
- Update `website/src/pages/docs/content/19-secret-scanning.md`, frontend demo mocks and relevant website demos/settings to reflect the actual shipped semantics. Coordinate with concurrent JWT-popup work rather than overwriting its components.

Acceptance: at desktop and mobile sizes a viewer can identify where the original is held, exactly what reaches the model, and what the file-write receives without reading the docs. Use synthetic values only; website demos must never invoke real credentials or provider requests.

## 8. Validation and completion

Core tests: repeated/different secrets, overlapping matches, unique scopes, literal token collision, same-client different conversations, cross-client denial, active-handle restrictions, Unicode/multiline/JSON escapes, empty values, one-pass restoration and no plaintext Debug/Serialize paths. Include a corpus showing known detector false negatives instead of implying universal coverage.

Protocol tests: split each token at every boundary; interleaved choices and tools; multiple tokens; malformed/foreign/expired markers; terminal-only arguments; duplicate completion events; strict schemas; disconnects and capacity errors. Assert no executable partial arguments and exact reconstructed JSON values.

Integration proof: a fake provider captures only placeholders; returns a file-write call; a temporary-directory test tool writes the exact synthetic secret. A tool-result echo is tokenized on the next provider turn. Cover local MCP and client-returned calls separately, normal and streaming paths, retries/fallback, guardrail/compression inputs, approval edits, cancellation, stored Responses and restart/expiry. Assert logs, monitor records, caches and SQLite contain no original for the protected exchange.

Website: verify desktop/mobile, light/dark theme, reduced motion, accessible reading order, consistent outbound/return handle and correct final synthetic file contents. Check no external runtime assets or secret-bearing examples. Test meaningful animation/interaction behavior only if introduced.

Run stable-toolchain workspace Clippy (all targets, warnings denied), fmt and workspace tests for Rust changes per AGENTS.md, confirming rustup compiler selection. Run desktop unit tests/build and `npx tsc --noEmit` for frontend/Tauri type changes; run `npm --prefix website run build:check` for shared/frontend/website inputs. Update every backend command, TypeScript request/response type and demo mock together if commands change. Run focused protocol/integration/browser tests in addition to these checks.

Mandatory final review: reconcile plan with code, review coverage, perform a fresh bug hunt, record supported paths and actual checks, then commit/push only task changes plus any automatically refreshed catalog. Use the configured signed user identity, normal push and merge commits for integration. Preserve concurrent work. Refresh only the owned PR-description section if a PR is involved. Do not dispatch hosted CI or a release/deployment merely to validate this feature.
