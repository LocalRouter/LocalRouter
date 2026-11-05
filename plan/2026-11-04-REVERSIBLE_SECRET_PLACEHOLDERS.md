- [x] Inspect existing scanning, redaction, session controls, tool execution and landing-page structure.
- [x] Specify placeholder format, isolation, lifecycle, response transformation and failure behavior.
- [x] Specify a landing-page section showing the complete secret/placeholder round trip.
- [x] Save and review this design; validate the documentation diff; commit and push only this plan.
- [x] Refine the approved XML tag/parser, OS storage, Redact action, caching and full-response restoration requirements.
- [x] Include a dedicated research paper, landing-page research card and expanded test matrix.
- [x] Implement the scoped OS-credential-store vault and reversible text/JSON transforms.
- [x] Wire request protection and response restoration into supported gateway codecs.
- [x] Protect MCP-via-LLM iterations and restore arguments before tool approval/execution.
- [x] Add explicit configuration, approval UI, types and demo support.
- [x] Implement the landing-page section, research paper/card and secret-scanning documentation.
- [x] Plan Review: reconcile implementation against every requirement and supported-path claim below.
- [x] Test Coverage Review: cover lifecycle, isolation, streaming, serialization, tools and failures.
- [x] Bug Hunt: review secret exposure, approvals, retries, caches, concurrency and protocol finalization.
- [x] Run appropriate stable Rust, frontend, website and focused integration checks.
- [ ] Commit and push validated implementation, preserving unrelated changes and including automatic catalog updates.

# Reversible secret placeholders

Status: implementation authorized 2026-11-04, including local tests, merge to master and a new release. Work occurs in the isolated feat/reversible-secret-redaction worktree. The checklist records completion as each step is verified.

- [x] Fetch origin and merge origin/master with --no-ff into the isolated implementation branch (already up to date at 99fa0143).
- [x] Run local end-to-end synthetic-provider/credential-store proofs and inspect the website preview.
- [ ] Merge the validated branch to master with --no-ff; push normally.
- [ ] Dispatch the next unused stable release; monitor builds and verify published assets and website.

## Goal and example

Keep selected secrets out of model-visible requests while allowing an assistant to reuse them verbatim in its answer or a tool call. LocalRouter retains the original value locally, sends an opaque but recognizable placeholder, then restores a recognized placeholder at the trusted destination, allowing the explicitly specified case and attribute variations below.

Example (synthetic secret only):

1. Client sends `Write API_KEY=demo-only-secret-value into .env`.
2. LocalRouter stores the original using the OS credential store and sends `Write API_KEY=<localrouter-redacted id="abc123" usage="Unmasked when returned verbatim"/> into .env`. The short `abc123` ID is illustrative, not the production entropy requirement.
3. Model returns that tag in an ordinary reply, structured response, or `write_file` content argument. JSON codecs escape attribute quotes correctly.
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
- `crates/lr-responses-sessions/src/lib.rs`: Responses continuation stores histories and cached responses in SQLite. Its disk persistence requires deliberate handling; storing originals in an OS vault does not make plaintext histories safe.
- `src-tauri/src/launcher/proxy.rs` applies existing secret redaction to provider-native HTTPS/WebSocket requests. Proxy response transformations require separate work; do not advertise reversible support there until implemented and tested.
- `crates/lr-api-keys/src/keychain_trait.rs` exposes `KeychainStorage`, `SystemKeychain`, `CachedKeychain` and mock implementations. Its automatic selection uses plaintext file storage for debug builds or an environment override; do not use that selection for this vault. Reuse OS-backed storage and `chunked.rs` for long values.
- `website/src/pages/Research.tsx`, `research-content.ts`, and `research/content/*.md` provide the existing research-paper registration, sections and Markdown loading. Register a new paper there and a card in Home's Research section.
- `website/src/pages/Home.tsx` already has a Safety Layer section and Secret Scanning demo. Add the round-trip explanation beside this story, using existing site typography, colors and component conventions.

## 1. Secret scan actions and compatibility

Extend the actual global/per-client secret-scanning action enum to `off | notify | redact | ask`, displayed as **Off / Notify / Redact / Ask** (increasing intervention). Keep the existing default, inheritance and serialized meanings of Off, Notify and Ask. Do not introduce a separate `secret_placeholders` toggle in addition to this action.

- Off: no new detection/replacement, preserving existing semantics. Already-issued placeholders still restore for active authorized exchanges; changing the scan mode cannot corrupt an in-flight response.
- Notify: existing notification behavior; it does not promise to withhold secrets from the provider.
- Redact: automatically protect eligible detected secrets with the reversible XML placeholders below, without an approval prompt for each finding.
- Ask: keep the existing approval flow and offer a clearly labeled reversible-placeholder choice, alongside existing allow/deny/irreversible-redaction decisions.

Keep existing irreversible Redact Once and remembered digest redactions distinct. A digest cannot reconstruct a secret, and existing destructive decisions must not silently become permission to restore it. Distinguish their labels/help text from the new reversible Redact mode. Define/test precedence: deny first, remembered irreversible redaction, new Redact behavior for remaining eligible findings, then the existing ask/allow/notify decisions. Existing detector exclusions/ignored values and system-message exclusions remain explicit; this protects selected/detected secrets, not every possible secret. Existing issued handles and known-value re-masking must not be exposed by later detector exclusions.

Update Rust config, validators/migrations, TypeScript `SecretScanAction`, exhaustive action maps, global and client controls, client inheritance summaries, approval decisions and demo mocks together. Explain the automatic Redact behavior and OS-storage requirement in UI help and docs. Update every related Tauri request/response type and command mock if the command shape changes.

## 2. Approved placeholder and tolerant recognition

Emit this exact canonical format (with a real random ID):

`<localrouter-redacted id="abc123" usage="Unmasked when returned verbatim"/>`

No system/developer instruction, manifest, extra message or per-turn preamble is injected. The tag itself explains reuse. This avoids changing unrelated cached prompt prefixes. Within an authorized conversation, keep the ID, emitted spelling, attribute order/spacing and usage text stable for a given secret, including retries, history replay, branches and provider fallback. Do not rotate existing history tokens on every call. Without a reliable conversation identity, guarantee request/retry stability only and document the limitation rather than sharing a vault globally to improve caching. Changing secret bytes creates a new reference; do not silently change an existing reference's value.

IDs contain at least 128 cryptographically random bits, encoded as 32 lowercase hex characters so case-insensitive lookup preserves the intended entropy. The short `abc123` examples are documentation fixtures only. Namespace ownership remains internal; no client name, original prefix/suffix, host, path or secret-derived digest is exposed. Compare IDs with ASCII case folding, and check collisions after normalization before storage. Never use case-distinct IDs as separate entries.

### Recognition algorithm

Use one bounded, quote-aware tag scanner, shared across ordinary text, structured content and stream adapters. Do not run a permissive regex over raw JSON or instantiate an XML engine with entity/DTD processing.

1. Search for **`localrouter-redacted` case-insensitively**. This spelling is canonical; misspellings such as `localrouter-redaceted` do not match.
2. Look backward to establish that the candidate starts at a literal opening `<` immediately followed by that tag name; no leading whitespace between `<` and the name, no `</`, namespace prefix, name suffix or enclosing unrelated tag. Require a name boundary (whitespace, `/` or `>`). Never search arbitrarily across adjacent tags/quotes to manufacture a match.
3. Look forward to the first unquoted `>` within the tag budget. Accept both `>` and `/>`, including whitespace/newlines around attributes or before the optional slash. A `>` inside a quoted attribute does not close the tag. Require a complete tag before replacing anything.
4. Parse attributes in any order. Match attribute name **`id` case-insensitively**, with optional whitespace around `=`. Accept single-quoted, double-quoted or a bounded bare ASCII ID value. Normalize ID value case only, not its content: no trimming inside quotes, prefix matches, fuzzy correction or secret guessing. Exact normalized ID lookup must succeed within this request's authorized scope.
5. `usage` is optional and not part of recognition. Its content, casing and presence do not matter; other well-formed attributes are ignored. Require exactly one nonempty ID; reject duplicate ID attributes (including different casing), malformed quotes and ambiguous/nested markup. Unknown attributes may contain quoted `>`; they must not confuse boundary recognition.
6. Replace the entire matched tag with the stored original in one pass. Do not rescan inserted secrets. Do not unescape XML/HTML entities, URL encodings or arbitrary model-created encodings in prose. Native JSON/transport escaping is decoded by its codec before recognition.

These all restore the same fixture value if `abc123` is authorized:

```xml
<localrouter-redacted id="abc123" usage="Unmasked when returned verbatim"/>
<LOCALROUTER-REDACTED ID="ABC123">
<LocalRouter-Redacted extra="ignored" Id = 'AbC123' />
<localrouter-redacted id=abc123>
```

The `>` form is accepted as a standalone placeholder marker; a closing `</localrouter-redacted>` is not itself a reference. This is an XML-style marker, not general XML document processing. Never consume surrounding prose, arbitrary element contents or neighboring tags as part of restoration. Specify a bounded candidate/attribute size using existing response/input limits and test adversarial overlong input; there is no whole-response scan-back or unbounded buffering.

Outbound replacement is one pass over original matched spans. Never rescan inserted markers. Merge overlapping detected spans to their exact original covered substring and register that entire value, preserving byte-for-byte round trips. The same value within a scope reuses one ID; separate scopes get unrelated IDs. Equality within a scope is intentionally visible.

## 3. OS credential storage, ownership and lifecycle

Implement the parser/transforms alongside irreversible `Redaction`, with a scoped vault service owned by the server. Depend on the existing `lr-api-keys` storage abstraction rather than building another secrets backend. Keep the parser independent of OS access for deterministic tests.

- Store each original in **macOS Keychain, Windows Credential Manager, or the project's configured Linux OS credential backend** through `SystemKeychain`/an explicitly OS-backed cache. Use a dedicated LocalRouter service/account namespace, separate from provider keys. Do not call `CachedKeychain::auto()` for this feature: debug mode or `LOCALROUTER_KEYCHAIN=file` must not make redacted originals appear in plaintext files. Unit tests use injected `MockKeychain`; native integration tests explicitly use the real OS backend.
- Persist the original successfully before sending its placeholder. Store failures, locked/denied/unavailable keychains and capacity errors return a local protection error; no fallback to plaintext network dispatch, plaintext files or memory-only storage. Use the existing long-value chunking layer for multiline private keys and credentials exceeding native entry limits.
- Bind entries to authenticated client identity plus explicit conversation identity, never to a bare caller-provided ID or monitor correlation ID. A request can resolve only references deliberately present in its model-visible context or explicitly carried by its authorized conversation. No global lookup by ID and no cross-client or cross-conversation restoration.
- Persist only the non-secret ownership/scope/ID/credential-key/lifecycle index needed for recovery, using the existing session persistence infrastructure with a narrow migration. Store sensitive per-entry data in the OS credential record; no raw secrets in settings, SQLite histories, monitoring, response caches, crash messages or logs. Authorize the recovered scope before keychain retrieval. Metadata must not be sufficient to rebind another client's entry.
- Without a reliable session identity, create a request-local scope. Full-history clients can re-supply originals, but cannot implicitly resolve another request's placeholders. Reuse within retries of that request is stable. Do not use MCP-via-LLM's hash-based session matching as vault authorization.
- Bind Responses continuation/retrieval and `previous_response_id` to verified ownership and durable scope metadata. A restart should recover a valid durable scope and retrieve its OS-stored originals; restart alone must not make persisted placeholders unusable. Missing, expired or revoked mappings produce explicit errors. Request-local scopes need not become indefinitely durable.
- Keep only necessary transient plaintext/zeroizing cache entries in process memory. Prevent original-bearing types from Serialize and plaintext Debug/Display; no claim of protection from a compromised process, swap or every serializer allocation. OS-backed storage is the source of truth, not a memory-only registry.
- Pin active streams/tools against cleanup. Reset, expiry, explicit forget and client deletion revoke access and remove associated OS entries/chunks and local indexes according to the existing session retention policy. Choose actual limits from current policy rather than inventing an operational timeout. Do not silently evict an active value.
- Handle interrupted writes, partial native chunk operations, restart and partial deletion. Persist lifecycle state sufficient to retry cleanup and collect orphans. A failed deletion revokes access immediately, reports the cleanup failure, and never claims that the OS copy is gone until deletion succeeds.
- Re-reading protected cached/stored responses applies authorization/restoration to the tokenized stored view using the OS vault. No restored copy is persisted as an optimization. Cached model-visible history stays byte-stable; never restore secrets before a provider replay.

## 4. Transform boundaries and correctness

Outbound: identify/authorize replacement, tokenize supported content without injecting instructions, then run all provider-backed auxiliaries on the protected view. Verify compression, decision routing, guardrail providers, fallback/retry and each orchestrator iteration cannot see the original. Keep any local approval view separate and privacy-preserving. Exact known-value masking also covers model-visible tool results and replayed history.

Inbound: restore placeholders in **all model-authored response content**, not only tool calls, after decoding provider wire formats and before final client serialization or local tool execution. Restore substrings such as `Bearer <handle>` and multiline file contents. Parse JSON-encoded `function.arguments`, recursively restore its string values and serialize properly; never raw-replace inside serialized JSON. Quotes, backslashes, CR/LF and Unicode in secrets must stay valid JSON and restore the exact string value. Handle content blocks, legacy function-call forms where supported and parallel calls by their actual identities.

Create an explicit endpoint/provider field inventory and test every response content path: assistant text, legacy completion text, visible reasoning/thinking, refusals, Messages/Responses text blocks, structured JSON output (including model-authored string keys where applicable), tool/function/custom-tool arguments, generated code, file contents, model-authored links/citation labels/artifact names, and final snapshots/retrievals. Restore anywhere within those content strings, including Markdown/XML/JSON code fences, rather than requiring the tag to be the whole field. For structured output, restore parsed values/keys then serialize; detect key collisions and validate the effective schema after restoration. For text containing code, restore the exact original value; do not invent shell/language escaping or claim a model can encode the unknown original safely.

Do not rewrite provider-owned protocol IDs, routing tool names, accounting, transport metadata, signatures, opaque/signed reasoning state or binary data. These are not model-authored response content. If a supported route exposes textual metadata/content not covered by the inventory, add its adapter/test rather than silently limiting restoration to `message.content` and tool arguments. Audio/image/binary generation and opaque provider execution cannot receive a textual replacement without a dedicated implementation. If sensitive content is found in an unsupported field, block or use the existing explicit user decision; never mutate it blindly or silently claim protection. Binary/PDF/image content and base64/URL-encoded variants outside supported decoding are not covered automatically. A model-side tool/hosted sandbox executes before LocalRouter sees its arguments: reject that combination when secret restoration is required, or require an implemented local execution path.

Schemas that demand a credential's exact pattern/length/enum can reject a placeholder. Detect known incompatible strict schemas before dispatch and return a clear error; do not send plaintext or weaken the user's schema silently. Bound schema validation and document unsupported cases.

LocalRouter-executed tools: restore to an execution-only copy, then run all relevant schema validation, permission, firewall and destination checks against the actual effective arguments. Show masked values in normal approval UI, with explicit local reveal if already supported. Recheck after user edits. Never approve one argument set and execute another. Keep the assistant/history view tokenized. Apply the same rules to background calls and parallel tools.

Client-executed tools: restore before returning the tool call. The caller receives the original and owns further approvals/execution. Generic text responses also restore recognized tags, as requested; do not restore unrelated provider-owned metadata.

Restoration does not authorize tool use. A model that can emit a known handle can ask a permitted tool to use that secret. Existing tool/destination approvals remain essential. Prompt injection can attempt this even though the model never receives the bytes; this feature prevents model-provider disclosure of selected secret values, not all exfiltration or unsafe tool behavior.

## 5. Streaming

Never replace independently in raw SSE/network chunks. Decode events first and transform logical text channels, keeping separate state per choice/content block/tool-call ID. A text transformer retains potential `<` plus partial case-insensitive tag-name prefixes, then a bounded candidate through quoted attributes until the unquoted closing `>`; a complete authorized tag emits the original once. This must support variants, not just prefixes of the exact emitted canonical tag. Unicode and SSE framing remain codec responsibilities.

For tool arguments in the first implementation, buffer the complete bounded argument JSON per call, validate/restore/serialize, then emit protocol-correct argument deltas and terminal snapshots containing the same restored value. This trades argument-stream latency for correctness; ordinary prose can continue streaming. Preserve tool identity/index ordering, stop reasons, usage, cancellation and errors. Reconcile terminal-only/missing argument suffixes through the existing completed-arguments handling before restoration. Never emit a partial tool call for execution.

Maintain both tokenized internal/monitor and restored caller views; finalize Responses/Messages terminal events and cached response reads consistently. Do not restore once in a delta and then duplicate the original from a terminal snapshot. Provider token usage describes the actual tokenized exchange, not the restored character length.

Unknown/wrong-scope/expired reserved tags in executable arguments cause an explicit local error before the call is returned or executed. A malformed reserved marker in executable arguments is also an error. In prose leave unknown or incomplete markers literal and expose a local diagnostic without a secret. Never guess a replacement. An interrupted tool-argument stream produces no executable call; a prose stream can only report errors using its protocol once headers have been sent.

## 6. Support rollout

1. Core OS-backed vault/parser/transforms and Chat Completions JSON plus streaming, including ordinary replies and client-executed file-write calls. Request-local scopes first; integrate existing explicit sessions without heuristic matching.
2. MCP-via-LLM JSON/streaming, background tools and every repeated provider dispatch; protect tool results. Reversible mode must reject automatic MCP orchestration until this phase passes.
3. Responses and Messages JSON/streaming, stored-response retrieval, previous_response_id, caches and continuity; legacy Completions separately. Audit System One and other scanner consumers; leave irreversible behavior there until a meaningful supported restoration path is defined.
4. HTTPS/WebSocket proxy provider-native response adapters as a separate deliverable. Leave passive/opaque traffic unsupported. Gate unsupported endpoints locally when a client has requested reversible protection; do not silently downgrade to plaintext or imply reversible restoration.
5. Land the four scan actions, UI/docs, public landing section and new research paper/card with the verified support scope. Do not publish feature claims before the corresponding backend behavior ships.

Ship the backend and user-facing settings together for the first supported gateway path; later phases expand the support matrix. Each protocol must pass the same test contract before it is enabled.

## 7. Landing-page section: “Keep the secret. Send the placeholder.”

Add a dedicated section adjacent to Safety Layer/Secret Scanning in `website/src/pages/Home.tsx`, backed by a small reusable `SecretPlaceholderDiagram.tsx`. Keep the existing detection/approval demo; the new section explains reversible reuse rather than duplicating that popup.

Suggested copy:

**Keep the secret. Send the placeholder.**

“LocalRouter holds the original value locally and sends a unique placeholder to the model. When the model copies it into a reply or tool call, LocalRouter puts the original value back.”

Supporting line: “A file-write call gets the real credential. The model sees the placeholder.”

Qualify the section with the shipped protocol scope, a link to `/docs/secret-scanning` and a “Read the research” link to `/research/reversible-secret-redaction`. Explain that selected/detected secrets are covered and tool permissions still apply. Accurately show originals held in OS credential storage, separate from provider API-key entries; do not claim that all secrets are caught or that tools never receive secrets.

Visual composition:

- Show Client / LocalRouter / Model as a left-to-right request path and a returning Model / LocalRouter / Tool path underneath. LocalRouter spans both rows, with a local “OS credential store” card connecting the replacement and restoration steps.
- Show a synthetic original `API_KEY=demo-only-secret-value` entering LocalRouter and staying in the local holding card. Only the colored placeholder chip crosses the clearly labeled model boundary.
- Return the exact same chip inside `write_file(.env, API_KEY=<placeholder>)`; at LocalRouter it becomes the synthetic original, and the final `.env` preview contains that original.
- Use one consistent accent for the handle, another for the held value. Label every state (“Original”, “Placeholder sent”, “Placeholder returned”, “Original restored”) so color alone carries no meaning.
- Use the approved `<localrouter-redacted id="abc123" usage="Unmasked when returned verbatim"/>` marker in the diagram and expanded code example. Short illustrative IDs must be marked as such. The holding card names Keychain / Credential Manager / Linux credential store appropriately. Explain that the generated form requests verbatim reuse while recognition tolerates name/attribute/ID case, omitted usage and optional self-closing slash.
- Use inline SVG/HTML/CSS for crisp diagrams, local assets and selectable text. No AI bitmap is necessary. Use established project icons; no invented provider logos, emojis, external assets or tracking.
- Optional subtle arrow/chip motion illustrates request then return. Supply a static complete diagram with prefers-reduced-motion, keyboard-friendly controls if interactive, a useful accessible description and a stacked mobile layout. Do not depend on animation to explain the feature.
- Update `website/src/pages/docs/content/19-secret-scanning.md`, frontend demo mocks and relevant website demos/settings to reflect the actual shipped semantics. Coordinate with concurrent JWT-popup work rather than overwriting its components.

Acceptance: at desktop and mobile sizes a viewer can identify where the original is held, exactly what reaches the model, and what the file-write receives without reading the docs. Use synthetic values only; website demos must never invoke real credentials or provider requests.

## 8. New research paper and landing-page research card

Create a dedicated paper **“Reversible Secret Redaction”**, slug `reversible-secret-redaction`, in the site's existing research system, not merely a docs paragraph or a separate marketing mockup.

- Add `website/src/pages/research/content/07-reversible-secret-redaction.md` (or the next unused prefix at implementation), using the established `<!-- @entry ... -->` section markers.
- Register the paper and subsection IDs in `website/src/pages/Research.tsx`. The existing `research-content.ts` glob loads its Markdown. Verify `/research/reversible-secret-redaction`, research index/sidebar, deep links, mobile navigation and previous/next navigation.
- Add a new card in **Home's existing Research section** linking to the paper, as well as a link from the new feature diagram/section. Use an established project icon, bundled assets and the current research-card style.
- Include Abstract, Problem and Threat Model, Placeholder Format, Case-Insensitive Recognition Algorithm, OS Storage and Scope, Cache/Conversation Continuity, Streaming and Full-Response Restoration, Tool Execution, Evaluation, Limits and Reproduction.
- Explain why a self-describing tag avoids extra system/developer instructions, and how stable canonical references preserve repeated prompt prefixes. Distinguish this property from actual cache-hit measurements; model/provider cache behavior remains provider-specific.
- Explain case-insensitive tag/name/ID recognition, optional usage/extra attributes and slash, JSON escaping, bounded tag parsing, native credential-store persistence/recovery and failure handling. Do not publish the obsolete exact-token-only or memory-only design.
- Reuse a small accessible round-trip figure. Include ordinary prose and structured/file-tool examples, not only tool calls. Discuss detection gaps, prompt injection/handle abuse, allowed tools receiving originals, signed/opaque fields and unsupported encodings.
- Report reproducible local evaluation using synthetic secrets and deterministic fake providers: round-trip fidelity, provider exposure checks, malformed/case/attribute variants, streaming fragmentation, native-store behavior and latency/memory observations where actually measured. Include commands, environment, sample counts and failures. Distinguish proposed design, automated test results and native platform verification.
- No fabricated success rates, unrun platform claims or invented model experiments. Paid model/provider tests require explicit per-action approval; all mandatory correctness tests should run without paid calls. Verify any external citations against primary sources when authoring the paper.
- Publish feature and research claims only for implemented/tested behavior. A design-only paper must clearly identify that status rather than imply the feature is shipped.

Acceptance: the landing page offers both the explanatory feature section and a discoverable new Research card; the complete paper renders with working anchors and accurate tested claims.

## 9. Extensive validation and completion

Build a table-driven regression corpus plus property-based tests/fuzzing for the scanner/transform boundary. Share fixtures between non-streaming and streaming tests. Mandatory coverage:

1. **Canonical round trips:** repeated/different/overlapping values; one-pass replacement; same-scope ID reuse; distinct scopes/values; literal marker collisions; Unicode, CR/LF, quotes, backslashes, empty candidates and long private keys. Arbitrary original strings must survive codec decode/restore/encode byte-for-byte as string values.
2. **Accepted marker variations:** tag-name casing; ID attribute casing and ID-value casing independently; optional or changed usage; extra attributes before/after ID; all attribute orders; single/double/bare ID quoting; tabs/newlines; spaces around `=`; `>` versus `/>`; quoted `>` in ignored attributes; multiple adjacent tags; tags embedded in prose, links, Markdown/code fences and tool content. All accepted variants resolve the same authorized fixture.
3. **Negative parsing:** misspelled `localrouter-redaceted`, longer/prefixed/namespaced names, closing tags, whitespace after `<`, absent `<` or `>`, missing/empty/duplicate IDs, mixed-case duplicate ID attributes, malformed/unclosed quotes, ID prefixes/extra whitespace, nested markup, unknown IDs, XML/HTML entity disguises, overlong tags and adversarial repeated near-matches. No cross-tag swallowing, regex backtracking blowups, panic, guessed match or accidental restoration of ordinary markup.
4. **Scope and lifecycle:** same-client different sessions, different clients with the same caller-supplied session label, copied/stolen valid ID in an unauthorized request, expired/revoked IDs, mode changes during a stream, retries/fallback, concurrent branches, active cleanup pinning, client deletion/reset, restart recovery for durable scopes and request-local teardown.
5. **Native storage:** mock-backed tests for store-before-send order, locked/unavailable/permission-denied native store, missing/corrupt value, chunked large/Unicode values, partial write/delete cleanup, failure retries and orphan cleanup. Assert neither debug defaults nor `LOCALROUTER_KEYCHAIN=file` selects a plaintext backend for this vault. Run native-store create/read/restart/delete smoke tests with synthetic fixtures on macOS and available Windows/Linux test environments; record which platforms were actually exercised. Tests own and clean only their dedicated service/account namespace, never real user credentials.
6. **All response content:** inventory-driven JSON/streaming tests for prose, completion text, visible reasoning/refusals, nested structured output and keys, parallel/legacy/custom tool calls, generated code/file contents, model-authored links/labels, Messages/Responses blocks and terminal snapshots/stored response reads. Preserve provider-owned metadata/signatures and validate post-restore JSON/schema/collision behavior.
7. **Every streaming split:** split the canonical tag and representative accepted variants at every byte boundary, plus randomized fragmentation. Interleave choices, text/reasoning blocks and parallel tool calls; handle arguments before headers, terminal-only arguments, duplicate final events, mixed SSE CRLF/Unicode frames, truncation, cancellation, client disconnects and failed native lookups. Assert reconstructed output matches non-streaming restoration, once only, and no executable partial arguments escape.
8. **Provider isolation and caching:** fake providers assert no selected raw secret reaches any dispatch, auxiliary guardrail/compression/classification call, retry/fallback or MCP loop. Capture emitted prompts to assert no instruction injection, stable canonical historical tags on subsequent turns and unchanged unrelated prefix bytes. Do not treat byte equality as proof of a hosted cache hit.
9. **Execution proof:** fake model returns a file-write tag, a temporary-directory test tool writes the exact synthetic original, and its echoed result is masked before the next provider call. Also test ordinary textual replies and client-executed tools. Exercise edited approvals, destination checks, blocked calls, background calls and effective-argument schema validation.
10. **Leak checks:** scan captured logs, monitor records, response/session SQLite rows, regular settings/files and caches for synthetic originals; only the designated mock/native credential store and explicitly restored caller/tool outputs may contain them. Verify safe Debug/Display/error paths. Include known detector false negatives so the coverage claim remains accurate.
11. **Four actions/UI:** config migration/defaults/inheritance and every Off/Notify/Redact/Ask choice; Redact automatically replaces; Ask decisions remain distinct; deny/remembered irreversible redaction precedence; mock type alignment and accessible labels.
12. **Website and research:** desktop/mobile, light/dark, reduced motion, accessible reading order, canonical tag examples and OS-storage label; exact same outbound/return marker and correct final synthetic contents; research card, paper route/index/sidebar/anchors and working diagram/docs links. No external runtime assets or real credentials.

Unknown/wrong-scope/expired or malformed reserved tags in executable output fail locally before tool dispatch/return. Prose retains unresolved/incomplete tags literally with a safe diagnostic; it never guesses or looks in another scope. Test bounded scan complexity and memory consumption on adversarial tag-like text against the established input/response limits, including stream finish/error paths.

Run stable-toolchain workspace Clippy (all targets, warnings denied), fmt and workspace tests for Rust changes per AGENTS.md, confirming rustup compiler selection. Run desktop unit tests/build and `npx tsc --noEmit` for frontend/Tauri type changes; run `npm --prefix website run build:check` for shared/frontend/website inputs. Run focused protocol, integration, parser property/fuzz and browser tests in addition to these checks. Do not claim these implementation checks ran as part of this documentation-only refinement.

Mandatory final review: reconcile plan with code, review coverage, perform a fresh bug hunt, record supported paths, native platform coverage and actual checks, then commit/push only task changes plus any automatically refreshed catalog. Use the configured signed user identity, normal push and merge commits for integration. Preserve concurrent work. Refresh only the owned PR-description section if a PR is involved. Do not dispatch hosted CI or a release/deployment merely to validate this feature.

## Implementation and validation record (2026-11-04)

The implementation uses one OS credential record per authenticated owner/scope, including its protected originals, and an OS-stored ownership/reference index. This replaces the proposed SQLite metadata migration: keeping both originals and recovery metadata in the existing native abstraction avoids an additional persistence mechanism. All native operations use SystemKeychain explicitly; mocks are injected into tests. Request-local records are deleted at completion. Durable scope expiry follows the existing Responses active-window policy; active requests are pinned, reset/client deletion revoke references, and failed deletions remain discoverable for cleanup.

The authenticated middleware protects before normal request capture, and restores after tokenized response capture. The narrow SecretContext capability travels with spawn_traced into automatic MCP loops. Restored execution arguments reach the existing permission/firewall path; gateway tool results are protected before being stored in the assistant conversation. Later router iterations scan new tool-result secrets and re-mask known originals. Ask requires a new explicit decision for new secrets discovered during an internal tool loop, rather than sending them silently. No system/developer instruction or manifest is injected.

The actual action controls are Off / Notify / Redact / Ask globally and Default / Off / Notify / Redact / Ask per client. Per-client Redact works while the global default is Off: startup and scanner rebuild account for client overrides, and client action updates rebuild the scanner. Existing irreversible redaction remains a separate action.

Supported return paths include Chat/Completions, Responses and Messages text and structured content, function/custom-tool arguments, streamed deltas and terminal snapshots, System One/decision JSON content, and MCP execution. Native Chat and Responses UTF-8 text attachments are decoded before masking and re-encoded. Provider-owned identities and signed/opaque reasoning remain untouched; a signed block containing a known raw secret is rejected. Hosted provider tools, unsupported media and proxy redaction are explicitly rejected rather than advertised as reversible. Strict schemas are never silently relaxed; constraints can be incompatible with a reference, as described in the research limits.

Local verification so far: all 81 workspace unit/integration targets passed using a streaming cargo artifact runner, and all 32 workspace doc-test targets passed together. The initial monolithic workspace command exhausted disk while linking, so task-generated test executables are removed after running. No other checkout's build artifacts or source files were removed. Rust stable 1.99.1 is selected explicitly through rustup sysroot PATH and RUSTC; debug symbols/incremental artifacts are disabled for disk capacity. Focused checks after final review cover 83 scanner tests (including a 2,000-input adversarial corpus and exhaustive representative streaming splits), the native macOS credential smoke test, ten gateway/boundary tests, and client/global configuration behavior. Windows/Linux native credential behavior has not been exercised locally.

Frontend verification: 148 unit tests, desktop build and website build:check passed; all 46 existing secret-popup browser regressions and six new landing/research/placeholder-choice browser tests passed, including light/dark modes. Desktop and mobile diagram screenshots were inspected. Browser tests use the already-installed Chromium binary. The research paper records only synthetic/local evaluation and claims no hosted-model success rates or cache-hit measurements.

Final Rust checks and release outcome are recorded below when complete. Release authorization is explicit in this conversation; no additional confirmation is required. Latest verified published release before dispatch is v0.0.173, with no competing release run; recheck before choosing the next tag.

Final local checks: workspace all-target Clippy with warnings denied and rustfmt check passed on stable 1.99.1. The final scanner suite passed 83 tests with the explicitly opt-in native test skipped in that run; the isolated native Keychain proof passed separately. All ten gateway tests and the global-Off/client-Redact configuration regression passed. A cleanup test initially shared an owner/scope with another concurrent mock test; isolated its synthetic owner and reran the suite successfully. Native test owners are unique per run and their value/index entries are deleted afterward. Existing block 0.1.6 future-incompatibility and frontend bundle-size notices remain informational.
