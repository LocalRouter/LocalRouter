# Streaming Request Pipeline (requests stream like responses)

Date: 2026-11-16 · Scope: lr-proxy (MITM HTTP + WebSocket + reverse proxy), lr-secret-scanner,
lr-monitor, src-tauri launcher firewall; native lr-server documented (stays buffered).
Predecessor: plan/2026-11-09-SECRET_SCAN_ROLLING_STREAMS.md (responses stream; requests
buffered because Deny/Ask must precede dispatch). Stopgap: 595d2e32 / 76745585 raised MITM HTTP
and WS caps to 256 MiB after a ~67 MB Codex conversation failed against 16 MiB / 64 MiB.

## Task checklist

- [ ] 0. Save this plan (copy-plan.sh) and keep this checklist current.
- [x] 1.1 Phase-1 prerequisites: fixtures + verification spikes (chunked upstream acceptance,
      real key order of Codex/Claude Code requests, hyper size_hint → Content-Length behaviour).
      Done with limits, see "Step 1.1 results": chunked acceptance shown at the edge only;
      Codex order from public source only; Claude Code order unverified; no captured fixtures.
- [x] 1.2 Phase 1a: header-time BodyPolicy; pass-through streaming for uninspected requests
      (MITM non-LLM paths, duplicate hops when not protecting, passive interceptor, reverse-proxy
      non-LLM paths); bounded monitor capture for them. See "Step 1.2 as built".
- [x] 1.3 Phase 1b: incremental JSON lexer + metadata sniffer (order-independent).
- [x] 1.4 Phase 1b: streaming content-decoding (zstd/gzip/deflate/br) with decoded-byte budget.
- [x] 1.5 Phase 1b: two-cursor lag-window pipeline (head detect, tail mask/emit), gates.
      Unit-granular, see "Steps 1.5–1.10 as built".
- [x] 1.6 Phase 1b: streaming detection with parity to extract_request_text + scanner.
- [x] 1.7 Phase 1b: streaming known-original masking (reuse encoding::rewrite_mask windows,
      signed/opaque/protocol-identity annotations, nested JSON strings, text-file parts).
- [x] 1.8 Phase 1b: Off/Notify/Redact wiring in AppFirewall via a new streaming policy API;
      Ask / model-Ask / guardrails / late-model-restricted keep the buffered path.
- [x] 1.9 Phase 1b: monitor: bounded capture, sniffer metadata, streaming Correlation hasher;
      remove repeated full parses; fix stale interceptor.rs doc (doc fixed in 1.2).
- [x] 1.10 Phase 1 tests (unit, parity, security invariants, TLS e2e, memory bound).
      Live Codex/Claude Code run not done (no network in this task).
- [ ] 2. Phase 2: WebSocket per-message streaming (frame-level decoder, incremental re-fragmenting),
      optional zstd re-encode of HTTP upstream body.
- [ ] 3. Phase 3: encrypted disk spill; Ask-mode hold-on-finding; late model with restricted
      models; model-Ask popup preview for oversized bodies; Notify/Ask popup excerpts.
- [ ] 4. Phase 4: guardrails text-projection hold (+spill); reverse-proxy LLM paths;
      response-side capture head+tail; WS server-message cap.
- [x] 5. Phase 5: native server memory/limit work (stays buffered; documented).
      See "Phase 5 as built".
- [ ] Plan Review: compare implementation against every item in this plan; implement gaps.
- [ ] Test Coverage Review: cover every new branch, error, fallback and cancellation path.
- [ ] Bug Hunt: reread with fresh eyes (off-by-one at window cuts, UTF-8/escape carry, races
      between head/tail cursors, cancellation, abort semantics, leaked plaintext).
- [ ] Local validation (rustup stable clippy -D warnings --all-targets, fmt --check,
      cargo test --workspace; frontend checks only if UI touched).
- [ ] Commit and push (Conventional Commits; normal push; preserve unrelated changes and
      include automatic catalog snapshot updates).

---

## 1. Inventory of every buffering point

Legend: "Cap" = the hard limit today. "Reasons" are the actual consumers found in the code
(verified against source; nothing listed that does not exist).

### 1.1 MITM proxy, HTTP request (crates/lr-proxy/src/transport.rs)

| # | Location | Cap | What is buffered |
|---|----------|-----|------------------|
| H1 | transport.rs:343-385 `Limited::new(body, MAX_REQUEST_BODY).collect()` | 256 MiB (`MAX_REQUEST_BODY = websocket::MAX_MESSAGE`, :43) → 413 | Entire raw request body, **for every path on a MITM'd host, including non-LLM paths, websocket upgrades and duplicate hops** |
| H2 | request_body.rs:134-147 `inspect` → `decode` :83-130 (`read_to_end`, :40-50; zlib :52-81) in `spawn_blocking` | 256 MiB decoded (`LIMIT`, :12), 4 encodings max | Second full copy: decoded body (only for `wire::detect` paths with Content-Encoding) |
| H3 | transport.rs:448-452 `request_body: inspected...to_vec()` | none beyond H1 | Third full copy into `ObservedExchange.request_body` (doc at interceptor.rs:77-81 claims "size-capped" — stale; it is the whole decoded body) |
| H4 | active.rs:94-101 (images JSON check), :136-141 `serde_json::from_slice` → `FirewallRequest.body` | none | Full `serde_json::Value` tree (several × byte size) |
| H5 | launcher/proxy.rs:471 `req.body.clone()`; documents.rs:203 `scan_projection` clone; text_extractor copies of all text; placeholders.rs:2046 `protect` candidate clone; :648 contract clone; proxy.rs:404-412 / 355 `serde_json::to_vec` | none | More full Value clones + re-serialized output |
| H6 | transport.rs:473/479 `base.request_body = Some(new_body.clone())`; :566 `Full::new(forward_bytes)` | none | Forwarded body copy + monitor copy |
| H7 | passive.rs:88-91 (`request_parts`, called at :252 emit_pending and again :318 complete / :373 record), :431-434 (`track_conversation`) | parse whole; stored capped (RAW_CAP 256 KiB :703, PARSED_CAP 64 KiB :717) | 3 further full parses of the request for monitor metadata/correlation |

Reasons H1–H7 buffer (all consumers of the full body on this path):
1. **Content decoding** (H2): policy inspection needs identity JSON; `decode` operates on complete
   `Bytes`. Codex zstd-compresses HTTPS fallback bodies (commit 76745585).
2. **Wire-format metadata** (active.rs:141 `wire::parse_request`): `model`, `stream`,
   `message_count`, `has_tools` (anthropic.rs:26-43, openai.rs:41-55/195-210).
3. **Images multipart vs JSON** (active.rs:94-101) — parses the body to decide.
4. **Client/strategy gate** (proxy.rs:240-245, 427-446): config only — *does not need the body*.
5. **Retrieval/control detection** (proxy.rs:255-280): path + method only.
6. **Uninspectable policy** (proxy.rs:150-206): rejects if `restricted_models` (:175-184) or
   `restricted_content` (guardrails, :189-197) — config + decode error only.
7. **Allowed-models** (proxy.rs:450-458): needs `model` only; must decide before dispatch (403).
8. **Rate limits** (proxy.rs:461, :99-116): metrics averages per strategy — *no body needed*.
9. **Secret scan** (proxy.rs:471-510 → pipeline.rs:1687-2014):
   - extraction of all texts (pipeline.rs:1756, text_extractor.rs:33+, role-aware; system texts
     skipped by label, engine.rs:57-66),
   - regex+entropy scan over all texts (pipeline.rs:1773),
   - remembered redact/ignore digests per finding (pipeline.rs:1803-1849),
   - time bypass / monitor intercept (pipeline.rs:1713-1719, client-level, no body),
   - **Redact**: register each finding in the vault (pipeline.rs:1890-1906) then mask whole body,
   - **Notify**: popup with whole masked request JSON + full-text context (pipeline.rs:1907-1924,
     2057-2091, 2144-2167) — non-blocking,
   - **Ask**: sequential per-finding popups, 60 s total deadline (pipeline.rs:2188-2312), Deny
     ⇒ 403 before dispatch; popup shows `request_json` = whole masked body (pipeline.rs:2067),
     `request_match` offsets into it (:2095-2142), and the *entire* text before/after (:2155-2157),
   - text-file attachments decoded from base64 for scan/mask (documents.rs:202-266, 274-285).
10. **Vault masking of known originals** (proxy.rs:385-386 → placeholders.rs:1722-1870): every
    string; skips objects with non-empty `signature`/`thought_signature` (visit_with_mode
    :2259-2267) and opaque `encrypted_content`/`encrypted_output`; leaves protocol identities
    (:2286-2294, depends on sibling keys via `is_protocol_object` :2188-2220); decodes nested
    JSON inside `content/text/output/input/arguments` strings (:2317-2325); authorizes existing
    references in history (:1835-1845) and opaque bindings (:1736-1748, SHA-256 key :80-87);
    diagnostics for signed/identity overlap (:1757-1829).
11. **Restoration contract** (proxy.rs:349/389 → placeholders.rs:634-706): tools schemas,
    `input[].additional_tools`, `response_format`/`text.format` schemas — needed only before the
    *response* starts, not before request dispatch.
12. **Extensions cleanup** (proxy.rs:393-403): removes `extensions.localrouter.session`.
13. **Guardrails** (proxy.rs:521-532 → pipeline.rs:1494, 1130-1253 → guardrails engine.rs:266-293):
    a safety *model* over all extracted messages; Deny ⇒ 403 before dispatch.
14. **Model-permission Ask** (proxy.rs:540-629): popup receives `Some(body.clone())` — the full
    request, **editable**; edited body is re-scanned (:592-619) and sent as `Replace`.
15. **Off/Notify with an existing client vault** (proxy.rs:325-363): re-serializes the whole body
    (`Protect{body: to_vec}`) only to attach a restore-only vault and contract.
16. **Monitor**: pending event metadata + capped raw/parsed body (passive.rs:246-307), conversation
    correlation (passive.rs:430-457 → lr-monitor conversation.rs:33-104: explicit session id,
    `previous_response_id`, SHA-256 of each message (≤2048), assistant flags, title from first
    user message), completion re-parse (passive.rs:318).
17. **Content-Length / representation headers** (transport.rs:395, request_body.rs:150-163):
    the forwarded length is derived by hyper from the `Full` body.
18. **Duplicate-hop**: header only (lib.rs `stamp_trace`; active.rs:86-88) — no body needed, yet
    still buffered.
19. **Usage tracking**: request *headers* + response headers/body (transport.rs:416-426, 653-657)
    — no request body.
20. **Retries/failover**: none on this path — one `send_request` (transport.rs:566-580); upstream
    connect happens only after the firewall (:532).

### 1.2 MITM proxy, WebSocket (crates/lr-proxy/src/websocket.rs)

| # | Location | Cap | What |
|---|----------|-----|------|
| W1 | `FrameDecoder::step` :214-224 waits until a *whole frame* is in `buf` | 256 MiB/frame (`MAX_MESSAGE` :38) → 1009 | Whole frame bytes + `to_vec` payload copy |
| W2 | fragment reassembly :260-280 (`frag`) | 256 MiB/message | Whole message |
| W3 | `assemble` :289-295 | — | UTF-8 validation of whole text message (keeps re-encode byte-exact) |
| W4 | `on_client_message` :424 full `serde_json::from_slice` just to detect `response.cancel`/`response.interrupt` | — | Full Value |
| W5 | :446 `payload.to_vec()` → interceptor (same H4–H7 chain: firewall parse/clone/protect/to_vec, monitor parses) and :496 cycle `request_body` clone | — | Several more copies |
| W6 | relay c2u :831-882 awaits the firewall per message; :851-860 re-encodes (`encode_message`, 1 MiB fragments, :43/:311-324) | — | Output copy |
| W7 | u2c :934-1003 server messages reassembled (W1/W2 caps), `ResponseRestorer::message` holds originals until carry flushes (≤16 MiB, secret-scanner wire.rs:686-692) | 256 MiB / 16 MiB | Whole server message |
| W8 | cycle capture :629-640 | 1 MiB (`CAPTURE_CAP` :52), keeps terminal event | Monitor |

Reasons: (a) per-message firewall = same consumers as 1.1 #2-#16; (b) control-message bypass
detection (W4); (c) byte-exact UTF-8 re-encode (W3); (d) replacing the message requires the new
payload (W6); (e) server side: per-message restoration, cycle routing by `response_id`/`item_id`
(:545-597), terminal detection (:538-540), `codex.rate_limits` usage events (:531-537).

### 1.3 Reverse proxy (crates/lr-proxy/src/reverse.rs)

| # | Location | Cap | What |
|---|----------|-----|------|
| R1 | :225-236 `Limited::new(body, MAX_REQUEST_BODY).collect()` | **16 MiB** (:37) → 413 | Whole body for **every** path (native `/api/*` incl. model pulls/blob uploads, `/v1/*`) |
| R2 | :265-266 `request_body::inspect` | 256 MiB decoded | decoded copy |
| R3 | :285-289, :296 `on_request` (ActiveInterceptor via `observed_from_reverse`, active.rs:191-193) | — | same chain as H4–H7 |
| R4 | :372 `Full::new(req_bytes)` | — | forward |

Reasons: same as 1.1 for LLM paths (Ollama `/api/chat|generate`, ollama.rs:42-48; OpenAI paths).
For non-LLM paths there is **no consumer**; buffering is incidental. Likely consequence (not
verified live): Ollama blob uploads (`POST /api/blobs/sha256:…`, used by `ollama create` from a
local GGUF) above 16 MiB fail with 413 through the reverse proxy.

### 1.4 Native API server (crates/lr-server)

| # | Location | Cap | What / reasons |
|---|----------|-----|----------------|
| N1 | lib.rs:465-467 `DefaultBodyLimit` + `RequestBodyLimitLayer` | 16 MiB (audio 25 MiB :485-486, image edits 50 MiB :496-497, files `MAX_FILE_BYTES+64K` :368-370, batches 32 KiB :350) | transport-level cap |
| N2 | secret_placeholders.rs:125-157 buffers inference POSTs (`/chat/completions`, `/completions`, `/responses`, `/messages`, `/systemone`, `/decisions`, :103-112) | 16 MiB (`LIMIT` :23) | parse (:158-180), conversation id (:181), vault continuation (:195-215), secret scan Deny/Redact (:258-278), protect (:279-282), contract (:283-285), follow-up detector for MCP tool loops (:289-295), re-serialize (:296-303). The over-budget branch (:132-143) forwards unscanned, but is effectively unreachable because N1 errors first — fragile if limits diverge. |
| N3 | gateway_json.rs:33-50 `Json<Value>` then normalizers | N1 | response-cache option validation, replay controls, document normalization (base64 files), upstream tools, session controls, raw-control rejection, then typed deserialize |
| N4 | pipeline.rs:2628-2944 `run_turn_pipeline` (typed `ChatCompletionRequest`) | — | upstream tools/documents projections (:2642-2688), validation (:2690), auto-router popup with full request (:318-376), strategy/model checks (:386-405), model-firewall popup with full editable request (:537-567), **rate limit token estimate over all messages** (:2315-2352), secret scan (:1552-1616), guardrails parallel/sequential (:2767-2808), **prompt compression** rewriting `messages` (:2817-2899, lr-compression), **decision routing classification** of the whole request (:2519-2530, :2837-2911), conversion (:2913) |
| N5 | messages.rs:125-150 | — | Anthropic Messages → Chat translation for MCP-via-LLM clients; `policy_projection` (:151) |
| N6 | responses.rs:193, 227-240 | — | Responses→chat conversion; `previous_response_id` history stitching (lr-responses-sessions) |
| N7 | router lib.rs:1846 `request.clone()` per attempt | — | failover/auto-routing retries need a replayable request |
| N8 | router response_cache.rs:108-134 (`to_string(&request)` :121) | — | cache key = hash of full request wire + config |
| N9 | lr-providers features/prompt_caching.rs:79-93 | — | breakpoints relative to the *end* of `messages` (second-to-last) |
| N10 | chat.rs:429/477/668 + lr-mcp-via-llm manager.rs:483/689 | — | MCP-via-LLM tool injection + context-management catalog compression; loops |
| N11 | providers serialize the typed request to the upstream (reqwest JSON) | — | outbound body is a fresh full serialization |

lr-memory: no request-body consumer (memory is exposed as MCP tools, lr-mcp-via-llm
gateway_client.rs:61-62/134-135). lr-context ("context management") only via MCP-via-LLM (N10).

### 1.5 Response side (all paths)

| # | Location | Cap | Behaviour / reason |
|---|----------|-----|--------------------|
| S1 | tap.rs:83-110 `TappedBody`, MITM `MAX_CAPTURE` 1 MiB (transport.rs:37, :705), reverse 1 MiB (reverse.rs:35) | 1 MiB head only | Monitor capture + usage-endpoint parsing (transport.rs:676-683). **Head-only** ⇒ for SSE > 1 MiB the terminal event carrying usage is probably dropped (tokens/cost missing). WS keeps the terminal event (websocket.rs:629-635). Not verified live. |
| S2 | protected_response.rs:98-102 masked wire tap | 1 MiB | monitor sees masked provider bytes |
| S3 | secret-scanner wire.rs:802-900 `restore_stream` | rolling; `LIMIT` 16 MiB (:6) on held bytes → passthrough (:836-842); SSE-vs-JSON detection holds ≤1 KiB (:843-849) | placeholder restoration (already streaming) |
| S4 | wire.rs:677-741 `ResponseRestorer::frame` | per SSE frame, ≤16 MiB held | per-frame restoration; abandons to originals on budget |
| S5 | protected_response.rs:25-44 | — | compressed responses are not restored (Accept-Encoding forced to identity, transport.rs:389) |
| S6 | WS u2c whole-message reassembly (W7) | 256 MiB | see 1.2 |
| S7 | native: messages.rs:216 `translate_non_streaming` 32 MiB; :43 error bodies 64 KiB; finalize.rs:53 non-stream JSON repair; chat.rs:505-573 streaming repairer; orchestrator_stream.rs:429-506 MCP tool-call accumulation; response cache entries ≤1 MiB (response_cache.rs:16) | various | inherent to translation/repair/tool execution |

### 1.6 Memory amplification (why 256 MiB is not "256 MiB")
A 67 MB Responses body on the protected HTTP path is held as: raw `Bytes` (H1), decoded copy if
compressed (H2), `to_vec` copy (H3), `Value` (H4, typically 3-6× bytes), `req.body.clone()` and
`scan_projection` clone (H5), extracted text copies, `protect` candidate clone (placeholders.rs:2046),
re-serialized output plus monitor copy (H6), and three more `Value` parses in the monitor (H7).
Estimated peak: several hundred MB up to > 1 GB per request (**not measured**; Phase-1 adds a
measurement test). The WS path adds W1+W2+W4 copies.

### 1.7 Checked and found not to be request-body consumers in the proxy
Request compression (lr-compression), response cache, prompt-caching adapters, JSON repair,
MCP-via-LLM injection, memory/context management and API translation exist **only** on the
native server. The proxy forwards the same API it receives (passive.rs:265-267 `client_api ==
upstream_api`). The proxy does no retries or failover. Response guardrails do not exist
(`scan_responses` is a deserialize-only shim, lr-config types.rs:2839-2841).

---

## 2. What each consumer actually needs

"Pre-dispatch" = must decide before any byte reaches the upstream.

| Consumer | Needs | Pre-dispatch? | Streaming strategy |
|---|---|---|---|
| Proxy auth / client enabled / proxy mode / permission Off | headers + config | yes | header-time |
| Rate limits (proxy) | metrics averages (proxy.rs:99-116) | yes | header-time |
| Rate limits (native) | token estimate over all messages (pipeline.rs:2321) | yes | running counter (native stays buffered anyway) |
| Duplicate hop | trace header | yes | header-time |
| Unsupported Content-Encoding | header (+ later corrupt-stream error) | yes / abort | header-time; mid-stream corruption ⇒ abort upstream |
| Images multipart vs JSON | Content-Type header suffices | yes | header-time |
| Wire metadata (model, stream, counts, has_tools) | top-level fields; counts by element | model: see below; others no | sniffer; monitor updated later |
| Allowed-models / restricted provider | `model` | yes | gate closed until `model` seen **only if** `restricted_models` (proxy.rs:175-184) |
| Model-permission Ask (no bypass/explicit allow) | `model` to decide if needed; **whole editable body** for popup | yes | whole-document ⇒ buffered/spill; oversized ⇒ preview-only popup (Phase 3) |
| Auto-router popup / decision routing / compression / prompt caching / translation / failover / response cache / MCP-via-LLM | whole typed request | yes | native only; stay buffered |
| Guardrails | all extracted text + role; model verdict | yes | whole text projection ⇒ hold + spill (Phase 4); stream only when guardrails would skip |
| Secret Redact | rolling window per string + container annotations (role, part type, signature, protocol keys, file mime) | secret bytes must never leave | lag window; tail masks with everything registered so far |
| Secret Notify | findings + context | no (non-blocking) | rolling; popup from held body (≤L) or bounded excerpt (>L) |
| Secret Ask / Deny | finding; popup context; per-finding decision | yes for secret bytes; Deny ⇒ request must not be processed | hold-on-finding gate (Phase 3); ≤L identical to today |
| Remembered redact/ignore, time bypass | per-finding hash / client state | — | per finding / header-time |
| Vault known-original masking | per string + annotations | secret bytes | tail cursor |
| Opaque/signed state | `encrypted_content` value hash; `signature` sibling | must not alter signed content | annotations; bounded capture of opaque values for binding |
| Restoration contract | tools / schema subtrees | before the *response* | bounded capture of those subtrees |
| Extensions cleanup | `extensions` object (small) | yes (it is rewritten) | hold top-level `extensions` value (bounded) |
| Monitor event | metadata + capped raw/parsed body | no | sniffer + bounded head capture |
| Conversation correlation | explicit ids, `previous_response_id`, per-message hashes, first user title | no | streaming canonical hasher |
| Usage accounting | headers + response | no | unchanged |
| Content-Length | final length | at header send | preserve if pass-through, chunked if transformed |
| Retries/failover | replayable body | — | none in proxy; native keeps typed clone |

---

## 3. Streaming design

### 3.1 Architecture: two cursors over a lag window
Per inspected request a `RequestPipeline` with:

```
client Incoming ─▶ Decoder ─▶ HoldBuffer (decoded bytes, offsets) ─▶ Tail cursor ─▶ Encoder ─▶ upstream Body
                                   ▲
                         Head cursor (lexer + sniffer + detector + annotations)
```

- **Head cursor** reads as soon as bytes arrive. It lexes the JSON, extracts metadata, runs
  secret detection, registers new findings (Redact) or raises gates (Ask), and annotates
  containers (object start offset → role, part type, signed?, protocol object?, file mime).
- **Tail cursor** re-lexes the same bytes and emits output. It masks known originals using the
  vault's state at release time, so a secret registered by the head anywhere in the next L bytes
  still masks earlier occurrences. Edits are mapped back onto the original byte stream; untouched
  spans are copied byte-for-byte. The response side already works this way (json_stream.rs
  raw-source queue).
- **Release rule:** a byte is released only when
  - it is at least L behind the head, or the input reached EOF;
  - no gate is closed; and
  - every container that encloses it has complete annotations.
- **L (lag)** defaults to 16 MiB, the current native and secret-scanner budget (`MAX_TAG_BYTES`,
  placeholders.rs:22). For bodies ≤ L nothing is released before EOF, so semantics match today
  exactly: the whole body is scanned and every decision is made before the first byte leaves.
  Only bodies > L stream.
- **Strict mode** (`lag = unlimited`) restores today's guarantee for every size, with disk spill
  added in Phase 3.

Residual risk, for bodies > L only: suppose a secret first becomes detectable more than L bytes
after an earlier bare occurrence (for example, a keyword rule fires only on the later
`password=...` context). The earlier occurrence has already been released. In practice the
client-scoped continuation vault (`Vault::find_client`, 14-day retention) already knows secrets
seen in earlier turns, so this needs a secret's first-ever appearance to fall in this shape inside
one request. Mitigation: keep 64-bit fingerprints of released string tokens of 8 or more bytes.
When a new registration matches one, emit `protection_diagnostic` ("an earlier occurrence was
forwarded before detection"). Document this in the UI for strict users.

### 3.2 Incremental JSON lexer, metadata sniffer, late `model`
- New byte-level lexer in lr-secret-scanner, generalized from `json_stream::JsonStream`
  (json_stream.rs:8-23, which already tracks containers, key, string/escape carry and UTF-8 carry).
  - Events: ObjectStart/End, ArrayStart/End, Key (bounded at 4 KiB), StringChunk (decoded
    text + raw span), StringEnd, Scalar (raw).
  - It tracks the path (`/input/12/content/3/text`), depth (bounded; over the bound ⇒ treat the
    subtree as opaque for metadata and fall back to whole-subtree hold), and surrogate pairs.
  - It rejects invalid UTF-8 the same way the response side does.
- `MetadataSniffer` collects these top-level fields regardless of order: `model`, `stream`,
  `type` (WS), `previous_response_id`, `client_metadata`, `metadata.user_id`, `extensions` (each
  captured with a bound, e.g. 64 KiB), plus element counts of `messages`/`input`, and `tools`
  presence and count.
- **Key order is not guaranteed.** Codex probably puts `model` first and `stream` after `input`;
  the Anthropic SDKs probably put `model` first and `system`/`tools` after `messages`. Neither is
  verified (step 1.1). Only gates need `model`:
  - If nothing needs `model` (allow-all strategy, permission not Ask), stream immediately. The
    monitor pending event shows "model: pending" and is updated when the sniffer sees it.
  - If `restricted_models`, or permission is Ask, the "model gate" stays closed until `model` is
    seen. Bytes are held in RAM up to today's 256 MiB cap (Phase 1, then 413 as today) or on disk
    (Phase 3).
  - `stream` is not needed before dispatch: the response is classified by Content-Type and
    `detect_sse`.

### 3.3 Header-time decisions (new `BodyPolicy`)
Add `ProxyInterceptor::body_policy(&RequestHead) -> BodyPolicy` (head = client, path, method,
headers, trace). `Firewall` gets the same method. Possible outcomes:
- `PassThrough`: forward `Incoming` untouched with its size hint and encoding, plus a bounded head
  capture for the monitor. Used when:
  - `wire::detect` returns None (non-LLM paths, including uploads on MITM hosts);
  - duplicate hop and `!protects_secrets`;
  - the passive interceptor;
  - reverse-proxy non-LLM paths.
- `Reject(status, msg)`: decided from headers alone:
  - client policy unavailable, disabled, or not in a proxy mode;
  - permission Off;
  - rate limit;
  - unsupported encoding while restricted (proxy.rs:150-203).
- `Stream(StreamPolicy)`: masking mode, gates (`needs_model`, `whole_document`), lag L, and
  whether a vault continuation exists (restore-only).
- `Buffered`: today's path, used for Ask secret mode, model Ask, active guardrails
  (`restricted_content` predicate at proxy.rs:189-197), and Images when Content-Type isn't JSON
  (Phase 1).

### 3.4 Redact mode (request path)
- **Detection (head):**
  - Build a projection text stream equivalent to `extract_request_text`: same labels, roles,
    `tool_use.input` serialization (`inspectable_tool_input`), and tool_result text blocks joined
    with "\n".
  - Feed it to `RegexEngine` in windows (64 KiB step, overlap ≥ the longest bounded rule span,
    e.g. 16 KiB for PEM blocks). Every rule's maximum span must be audited (step 1.6); unbounded
    rules get an explicit cap with a diagnostic when it is exceeded.
  - Skip system texts (`!scan_system_messages`) by path (`system`, `instructions`) or by the
    message `role` annotation. If `role` arrives after content, hold the scanner text for that
    message (bounded) until `role` is known.
  - Each finding goes through remembered redact/ignore digests (pipeline.rs:1803-1849), then
    `context.register` (pipeline.rs:1890-1906). The `record_secret_scan` event is emitted once
    at EOF with the aggregated findings, excerpts limited to 120 chars as today (:1966-1979).
- **Masking (tail):** reuse the window logic of `encoding::rewrite_mask` (encoding.rs:22-90:
  64 KiB step, lookahead clamped to 16 MiB, commits only at boundaries outside quotes and matches)
  per string value as it streams.
  - Known originals are matched in raw and JSON-escaped spellings (as `redact_monitor` does,
    placeholders.rs:1928-1943), so nested-JSON strings (`arguments`, `content`) are covered
    without a nested parse.
  - When a string is small enough (≤ 1 MiB), keep the exact nested decode path
    (visit_with_mode :2317-2325) for parity.
  - Indivisible quoted views that exceed the window keep the existing bounded policy-body
    fallback and its diagnostic, unchanged from today.
- **Annotations that block tail release until known** (bounded; on overflow, hold and spill, or in
  Phase 1 fall back to treating the subtree as opaque, pass it through unchanged and emit a
  diagnostic, which is exactly today's behaviour for signed content):
  - the object has a non-empty `signature`/`thought_signature` ⇒ the whole object passes
    unchanged (today: visit_with_mode :2259-2267; plus `signed_secret` diagnostic :1757-1789).
    Anthropic thinking blocks put `signature` after `thinking`, so the hold is needed.
  - `is_protocol_object` sibling keys (placeholders.rs:2188-2220) ⇒ protocol identities
    (`id`, `call_id`, `type`, …) are never masked.
  - `encrypted_content`/`encrypted_output` ⇒ opaque; also streamed into the opaque binding
    hash `key("opaque", value)` (SHA-256 over serde's `("opaque", value)` encoding; reproduce
    serde escaping incrementally) for capability binding (placeholders.rs:1736-1748).
  - `file`/`input_file` parts ⇒ text files are base64-decoded, masked and re-encoded
    (documents.rs:274-285). Phase 1: hold file parts ≤ 8 MiB; larger ones pass through with the
    existing "Document inspection unavailable" diagnostic.
- **Existing references** in history authorize restore IDs (placeholders.rs:1835-1845): the head
  adds them to `allowed` as it sees them; the response starts only after EOF.
- **Contract:** capture `tools`, `input[type=additional_tools].tools`,
  `/response_format/json_schema/schema` and `/text/format/schema` subtrees (bounded, e.g. 8 MiB
  total). Call `set_request_contract` with a synthetic Value at EOF; over the bound, keep the
  existing "contract unavailable" diagnostic.
- **Extensions:** hold the top-level `extensions` value (bounded) and emit it without
  `localrouter.session`; drop the key if it becomes empty (proxy.rs:393-403).

### 3.5 Off / Notify
- Off with no vault, or a duplicate hop when not protecting ⇒ `PassThrough` (original bytes,
  length and encoding).
- Off/Notify with a continuation vault ⇒ forward the **original bytes untouched**. Run the head
  pass only for contract capture, existing references and opaque bindings, then attach a
  restore-only vault. This removes the full re-serialization at proxy.rs:325-363.
- Notify ⇒ tee: original bytes flow immediately, and detection runs on a decoded copy.
  - Popup at EOF. For bodies ≤ L, build today's popup by parsing the retained copy.
  - For > L, send findings with bounded context (64 KiB before and after within the same text)
    and `request_json: None`. The popup UI's handling of `None` must be checked (step 1.8).

### 3.6 Ask / Deny (Phase 3)
- Bodies ≤ L: identical to today (nothing released before EOF; sequential popups,
  pipeline.rs:2188-2312).
- Bodies > L, "hold-until-decision": when the head finds an unremembered finding, close the
  `secret` gate at the finding's container start, so the tail can't release anything from there.
  - The client keeps uploading; held bytes spill to disk beyond the RAM threshold (§3.13).
  - The popup shows the finding with bounded context, waiting up to 64 KiB past the finding or
    to the end of its text.
  - The existing 60 s deadline and fallback-to-redaction behaviour still apply.
  - Decisions:
    - Allow / AllowOnce / bypass ⇒ reopen.
    - RedactOnce ⇒ add to the request-local `Redaction` plan, applied at the tail.
    - ReplaceSecrets / ContinueWithRedaction ⇒ register, then reopen.
    - **Deny** ⇒ abort the upstream: drop the hyper request body sender with an error so hyper
      resets the connection, which guarantees the request is never completed or processed. Answer
      the client 403 `secret_scan_denied_by_user`. No secret byte was sent, though earlier bytes
      were.
- Multiple findings are queued and asked sequentially, as today.
- Deny on bodies > L means "partial non-secret prefix was transmitted but not processed". This is
  a semantic change to document (the user explicitly accepts this for streaming). Strict mode
  avoids it.

### 3.7 Model selection and permissions with a late model
- The proxy does no routing or model rewriting. `model` matters only for the allow-list, Ask and
  metrics labels.
- Allow-all and not Ask ⇒ no gate.
- Restricted ⇒ model gate (§3.2). After `model` is known, evaluate `is_model_allowed`; on deny,
  close permanently ⇒ 403 with nothing sent, because the gate was closed from the start.
- Model Ask without a bypass:
  - Phase 1 ⇒ `Buffered` (today's popup with an editable full body).
  - Phase 3 ⇒ hold and spill, plus a popup with metadata and a truncated, read-only preview when
    the body exceeds the popup's edit limit (e.g. 1 MiB).
  - On approval, open the gate; on deny ⇒ 403 with nothing sent.
- The native server keeps full routing (auto-router, decision classification, strategy) buffered;
  see §3.14.

### 3.8 Content-encoding
- Streaming decoders:
  - zstd: `zstd::stream::raw::Decoder`, `window_log_max(24)` as today;
  - gzip: `flate2::Decompress` with gzip header handling for multi-member streams (today
    `MultiGzDecoder`, request_body.rs:121);
  - deflate: the zlib wrapper (request_body.rs:52-81 already chunked);
  - br: `brotli::DecompressorWriter`;
  - stacks of up to 4 encodings, as today.
- Run in small batches inline, or `spawn_blocking` per batch over 256 KiB.
- Budget: total decoded bytes ≤ configurable cap (default 1 GiB) plus an expansion-ratio check
  (zip bomb). Exceeding it ⇒ abort the upstream with 413.
- Output:
  - `PassThrough` and Off/Notify tee ⇒ original encoded bytes and original headers.
  - Transformed (Redact/Protect) ⇒ identity, dropping representation headers
    (`request_body::rewritten`, request_body.rs:150-163), as today.
  - Phase 2 option: re-encode zstd streaming when the client sent zstd (saves upload bandwidth
    for 67 MB conversations).
- Corrupt input mid-stream ⇒ abort the upstream (no secret was sent, because only decoded and
  masked data was released) and answer 400 if headers haven't been sent, otherwise just reset.

### 3.9 Content-Length vs chunked
- PassThrough / tee ⇒ pass `Incoming` (exact size hint) so hyper keeps a Content-Length (verify,
  step 1.1).
- Transformed ⇒ unknown length ⇒ HTTP/1.1 `Transfer-Encoding: chunked`.
- **Unverified:** whether api.openai.com, chatgpt.com and api.anthropic.com accept chunked request
  bodies (spike in step 1.1). If one rejects (411/400), add a per-host "length-required" fallback:
  hold the transformed output (RAM ≤ L, then spill) and send it with Content-Length at EOF. That is
  still no RAM cap once spill exists.
- If the upstream responds early (401/413) while the client is still uploading, hyper returns the
  response; we stop reading the client and close after the response.

### 3.10 WebSocket (Phase 2)
- Replace message-level `FrameDecoder` output with frame-level events:
  `DataStart{opcode}`, `DataChunk{bytes}` (unmasked incrementally, mask offset carried across
  reads, no wait for the whole frame), `DataEnd`, and control frames.
- Each client data message gets its own `RequestPipeline`.
  - The head sniffs `type` for control messages (`response.cancel`, content-free
    `response.interrupt`, websocket.rs:431-439) within a small bound, replacing the full parse
    at :424.
  - The tail output is re-fragmented: first frame with the data opcode, continuations, FIN
    (reuse `push_frame`, :326-350); frames are emitted as output becomes available. Text frames
    may split UTF-8 inside a message (RFC 6455 validates per message, :306-310).
  - Control frames from the client (ping/pong) are forwarded between continuation frames.
- Messages ≤ L are fully held, so firewall Reject keeps today's behaviour: a synthesized error
  event, with the connection staying open (:861-869).
- Mid-message Deny or abort for messages > L, after fragments were sent: send close 1008
  (policy) upstream so the partial message is never completed, reply to the client with the error
  event, then close 1011. Codex reconnects or falls back. Concurrent cycles on the same socket are
  also ended (rare for Codex); record them with an error.
- Keep `MAX_MESSAGE` as the per-message spill cap rather than a RAM cap. Server→client messages
  stay message-assembled (they're small events) with a lower cap, e.g. 64 MiB (Phase 4).

### 3.11 Retries, failover and connecting upstream
- The proxy has no retries today. With streaming, open the upstream TLS connection concurrently
  with head processing; today it opens after the firewall (transport.rs:532).
- If connect or handshake fails before any byte is released, retry once transparently (nothing
  was sent, so this is safe).
- After release, no replay: answer 502 and the client retries (Codex does).
- An optional spill-backed replay buffer is deferred; it isn't needed for Phase 1.
- Native failover (router lib.rs:1846) stays typed and in memory.

### 3.12 Monitor capture and correlation
- `ObservedExchange` gains `request_capture: BoundedCapture` (head ≤ RAW_CAP 256 KiB of the
  **forwarded** (masked) bytes, plus total length and a truncation flag) and
  `request_meta: Option<RequestMeta>` filled by the sniffer.
- `PassiveInterceptor::request_parts` uses `request_meta` when present; it parses the capture
  only if it isn't truncated, so a truncated capture is shown as raw text with the existing
  marker, consistent with today's PARSED_CAP behaviour (passive.rs:701-729).
- `Correlation`: add `Correlation::from_parts(explicit, previous_response, history_hashes,
  assistant_flags, title, reset)` and `MonitorEventStore::track_conversation_with`.
  - The tail computes per-message SHA-256 of the canonical compact serialization **byte-identical
    to `serde_json::to_writer(Value)`** (conversation.rs:25-30): key order preserved
    (`preserve_order` is on workspace-wide via lr-providers/lr-batches), serde string escaping,
    numbers re-formatted through `serde_json::Number`.
  - Parity tests are required. Over 2048 messages ⇒ no history, as today (:49-50).
  - Title = first user content (160 chars).
- Websocket cycles store the same bounded capture instead of `request_body` (websocket.rs:496,
  :664).
- This also removes the repeated full parses (H7, W4).

### 3.13 Spill buffer (Phase 3)
- `SpillBuffer`: RAM up to a threshold (L by default), then an anonymous temp file
  (`tempfile::tempfile()`, unlinked or delete-on-close) in a 0700 dir under
  `lr_utils::paths::config_dir()/proxy/spill`.
- **Encrypted** with a per-request random key held only in memory (ring ChaCha20-Poly1305,
  chunked AEAD with chunk counters as nonces), because held bytes may contain unredacted secrets.
- Zeroize keys on drop. Global in-flight spill budget (e.g. 4 GiB) plus per-request cap
  (e.g. 2 GiB) ⇒ 413 / 1009.
- Never used for passthrough. Privacy policy: local-only, no new network access.

### 3.14 Must stay buffered, and the bounded fallback for each
| Feature | Why | Fallback |
|---|---|---|
| Native API translation (messages↔chat↔responses, N5/N6) | typed conversion of the whole conversation | stays buffered; Phase 5 memory reductions |
| Response cache key (N8) | hash over the whole typed request + config | could be streamed but needs the typed request anyway; native only |
| Prompt compression (N4), prompt caching (N9), decision/auto routing (N4), MCP-via-LLM + context management (N10), Responses sessions (N6), failover clones (N7) | whole-conversation rewrites, classification, replay | native only; buffered |
| JSON repair (S7) | needs complete content for non-stream; streaming repairer already exists | response-side; unchanged |
| Guardrails (proxy + native) | safety model verdict over all text, Deny before dispatch | Phase 4: hold only the text projection plus gate the body (spill), no full Value; optional opt-in "withhold-tail": forward everything except the final bytes and abort on deny (content reaches the provider unprocessed, so it is off by default) |
| Model-Ask popup with editing | user edits the full body | hold + spill; preview-only above the edit limit |
| Secret Ask popup with full `request_json` | UI shows the whole masked request | ≤L as today; >L bounded excerpt |
| WS server messages | per-event restoration, routing | per-message assembly, lower cap |

---

## 4. Phased implementation plan

### Phase 1 (smallest high-value; MITM HTTP only)

1.1 Spikes and fixtures:
1. Capture or anonymize request fixtures from installed Codex (HTTPS fallback with zstd, and
   WS `response.create`) and Claude Code. Record the real key order.
2. Build synthetic generators for 50/300 MiB conversations.
3. Spike chunked uploads against each real upstream with a harmless request (model list or a
   tiny request) using an isolated test account; record the result in this plan.
4. Confirm that hyper keeps Content-Length for an exact `size_hint`.

1.2 Phase 1a, PassThrough:
- transport.rs: compute `BodyPolicy` before line 343. For `PassThrough`, skip the collect and pass
  `Incoming` (wrapped in a capped tap for the monitor) into `send_request`.
- Same in reverse.rs:225 for non-LLM paths.
- Interceptor/Firewall traits: add `body_policy` with a default of `Buffered`, so behaviour is
  unchanged until it's implemented.
- Fix the interceptor.rs:77-81 doc.
- Tests: a 600 MiB upload on a non-LLM MITM path is streamed (the upstream receives its first byte
  before the client finishes) with constant memory; the reverse proxy forwards a 64 MiB
  `/api/blobs` upload.

1.3 Lexer and sniffer in lr-secret-scanner, a new `request_stream` module, with every-byte
    fragmentation tests and key-order permutations.

1.4 Streaming decoders (lr-proxy `request_body.rs`): keep the batch API for `Buffered` and add
    `StreamDecoder`.

1.5 `RequestPipeline` (lr-proxy, new `request_stream.rs`): HoldBuffer (RAM, capped at today's
    256 MiB in Phase 1), head/tail cursors, gates (model, annotation), lag L, abort semantics,
    cancellation (client disconnect ⇒ abort the upstream and close the pending event through the
    existing `CancellationGuard`, transport.rs:516-529).

1.6 Streaming detector: projection parity with `extract_request_text`, a rule-span audit,
    remembered digests, Redact registration, and an aggregated `record_secret_scan`. Expose a
    scanner-level API from lr-server so it doesn't depend on `serde_json::Value` (pipeline.rs
    needs a refactor: split `scan_request_for_secrets_fresh` into "policy context" and
    "finding handling").

1.7 Streaming masker: annotations, rewrite_mask windows, escaped spellings, opaque binding hash,
    text-file parts (bounded), contract and extensions capture.

1.8 Wiring (src-tauri launcher/proxy.rs):
- `AppFirewall::body_policy` returns `Stream` for Off/Notify/Redact when all of these hold:
  - guardrails would skip (proxy.rs:189-197 predicate);
  - permission is not Ask, or there is a valid bypass, explicit allow or time approval (the
    proxy.rs:540-553 logic, evaluated when `model` arrives);
  - restricted models ⇒ model gate.
- Ask ⇒ `Buffered`.
- Off with a vault ⇒ tee + restore-only.
- Check the popup's handling of `request_json: None` for Notify on bodies > L (UI file to verify:
  the secret-scan popup component under `src/`).

1.9 Monitor changes (§3.12) in lr-proxy passive.rs and lr-monitor.

1.10 Tests (§6). Raise no caps; Phase 1 never behaves worse than today.

### Phase 2: WebSocket streaming and upload re-encoding
§3.10 in websocket.rs (decoder events, per-message pipeline, re-fragmentation, abort via 1008),
plus an optional zstd re-encode for HTTP transformed bodies.

### Phase 3: Ask mode, spill, late model
SpillBuffer (§3.13); hold-on-finding Ask (§3.6); model-gate spill; model-Ask preview popup;
Notify/Ask excerpt popups. UI and Tauri types if the popup payload changes (follow the AGENTS.md
Tauri command sync checklist: Rust struct, `src/types/tauri-commands.ts`, demo mock).

### Phase 4: Guardrails, reverse-proxy LLM paths, response-side capture
- Guardrails text-projection hold.
- Reverse-proxy LLM paths (Ollama NDJSON chat/generate, OpenAI paths) through the pipeline.
- `TappedBody` capture keeps head + tail (or parses usage incrementally) so the terminal SSE
  usage survives responses > 1 MiB (S1).
- Lower the WS server-message cap.

### Phase 5: Native server (stays buffered)
Make the 16 MiB limit configurable, add an in-flight large-body semaphore, parse with
`serde_json::from_reader` over spill-backed bytes, avoid the `Bytes`+`Value`+typed triple copy in
secret_placeholders.rs and gateway_json.rs, and make the unreachable fail-open branch
(secret_placeholders.rs:132-143) fail closed for masking modes.

---

## 5. Risks

- **Security:**
  - Never send an unredacted secret. The invariant applies to the bytes the upstream receives,
    including aborted partial uploads.
  - Residual risk for bodies > L: an earlier bare occurrence (§3.1) — diagnostic plus strict mode.
  - Annotation overflow must fail toward "hold" (or today's documented opaque passthrough), never
    toward "release unmasked unknown context".
  - Spill files must be encrypted and unlinked.
  - Aborts must reset the connection, not send a valid FIN-terminated body.
  - Deny on bodies > L has sent a non-secret prefix: document it.
- **Protocol:**
  - Chunked acceptance by upstreams (unverified).
  - HTTP/1.1 early responses.
  - WS fragmentation with interleaved control frames.
  - UTF-8 split across fragments.
  - Content-Encoding stacking.
  - Exact escape spelling preservation for untouched spans (better than today, which
    re-serializes the whole body).
- **Signed/encrypted fields:** signature-after-content ordering, `encrypted_content` binding
  hashes must match the batch `key()` exactly, and nested JSON in strings.
- **Parity:** streaming detection must find the same findings as batch on the corpus; joined-text
  cases (tool_result blocks joined with "\n") need special handling; correlation hashes must be
  byte-identical.
- **Performance:** the head and tail both lex, roughly 2× lexing CPU; the windowed regex must not
  regress (the earlier plan found quadratic overlay work).
- **UX:** popups for oversized bodies change (excerpts); the Ask hold may exceed client timeouts
  (popup deadline 60 s, model Ask 120 s).
- **Uncertain:** the actual peak memory today (not measured), the Codex/Claude key order, the S1
  usage loss, the R1 Ollama blob failure.

---

## 6. Test strategy

- **Lexer/sniffer (unit, property):**
  - random JSON × random chunking (including every-byte), with output equal to input when nothing
    is masked;
  - permutations of key order (`model` last, `signature` after `thinking`, `role` after
    `content`);
  - escapes, surrogate pairs, invalid UTF-8, deep nesting, oversized keys.
- **Parity:** for the corpus (Codex/Claude fixtures plus the existing scanner fixtures in
  pipeline.rs and launcher/proxy.rs tests), compare streaming against batch:
  - findings;
  - masked output (parsed Value equality, and byte equality outside masked strings);
  - contract;
  - opaque bindings;
  - correlation hashes.
  Run at L ≥ body size (must be identical) and at small L, e.g. 4 KiB, to force streaming.
- **Security invariants:** an upstream test server records every received byte, even after
  aborts. Assert that no registered or detected original appears in any encoding (raw,
  JSON-escaped, nested-escaped, base64, percent, HTML) across:
  - chunk, TLS-record and WS-fragment splits;
  - a secret first appearing after L;
  - a secret split across a window cut;
  - signed blocks with signature-last;
  - system role-last;
  - text-file parts;
  - Deny mid-stream (Phase 3).
  Also assert the diagnostic for the "earlier occurrence" case.
- **Protocol e2e (crates/lr-proxy/tests/mitm_e2e.rs, real TLS, codex-client dev-dependency):**
  - 300 MiB request (above the old cap) forwarded with Redact;
  - zstd/gzip/br streaming decode including corrupt, truncated and bomb inputs;
  - chunked upstream;
  - Content-Length preserved on PassThrough;
  - early upstream 401;
  - client abort ⇒ upstream aborted and pending event closed;
  - upstream abort mid-upload;
  - WS incremental fragments (the upstream sees the first fragment before the client sends the
    last; ping interleaved; 1008 on deny).
- **Memory bound:** an instrumented pipeline counter (max held bytes) asserting ≤ L + window for a
  500 MiB body; an ignored benchmark measuring RSS before and after.
- **Regression:** the whole existing suite, including passthrough_e2e, reverse_e2e and
  codex_search_e2e.
- **Live:** installed Codex and Claude Code through an isolated file-mode debug app
  (`LOCALROUTER_KEYCHAIN=file`, no native Keychain), a long session over 67 MB, Redact with a
  synthetic credential; verify upstream masking via a TLS mock and real runs, as in the
  2026-11-09 plan.

## 7. Validation and mandatory final steps
- Local: `rustup run stable cargo clippy --workspace --all-targets -- -D warnings`,
  `rustup run stable cargo fmt --all -- --check`, `rustup run stable cargo test --workspace`
  (stable rustc confirmed). Frontend `npm run test:unit` and `npm run build` (plus the website
  build check) only if the popup UI changes (Phase 3).
- **Plan Review:** walk §1–§4 item by item against the implementation; implement anything missed.
- **Test Coverage Review:** every new branch, fallback, abort, cancellation and error path has a
  test.
- **Bug Hunt:** fresh reread for off-by-one errors at window, frame and UTF-8 boundaries, races
  between the head/tail cursors and gate reopen, state after abort, leaked plaintext in
  diagnostics or monitor, and spill key lifetime.
- **Commit and push:** Conventional Commits (e.g. `feat(proxy): stream uninspected request
  bodies`), a normal push to the configured upstream, preserving unrelated changes and including
  automatic `modelsdev_raw.json` updates.

## Step 1.1 results (2026-11-21)

**Chunked request bodies.** Probed each upstream directly (not through LocalRouter) over
HTTP/1.1 with an unauthenticated, content-free `POST`: body `{}` with `Content-Length`, the same
with `Transfer-Encoding: chunked`, and a 200 KB whitespace-padded `{ }` sent chunked.

| Upstream | Content-Length | chunked `{}` | chunked 200 KB |
|---|---|---|---|
| `api.openai.com/v1/responses` | 401 | 401 | 401 |
| `chatgpt.com/backend-api/codex/responses` | 401 | 401 | 401 |
| `api.anthropic.com/v1/messages` | 401 | 401 | 401 |

Re-run on 2026-11-21 for the `{}` cases (direct, `--noproxy`, generic User-Agent, no
credentials): same six 401s.

All nine answered with the ordinary authentication error and no 411/400 about length, and the
chunked 200 KB bodies were accepted in full before the answer. This shows the edge of each host
accepts chunked HTTP/1.1 requests. It does **not** show that the authenticated path does: the 401
is produced before the request is routed to the model backend, which may impose its own framing
rules, and no body near the sizes that matter (tens of MiB) was sent. Treat chunked acceptance as
likely, not proven; the per-host "length-required" fallback of §3.9 stays in the design until a
transformed request has gone through with real credentials (step 1.10 live run).

**hyper and Content-Length.** Confirmed by test
(`tap::tests::hyper_keeps_content_length_for_an_exact_size_hint`): hyper's HTTP/1.1 client sends
`Content-Length` for a body whose `size_hint` is exact and `Transfer-Encoding: chunked` for one
whose hint is unknown. `Incoming` reports an exact hint for a request that had a Content-Length,
so pass-through keeps the client's framing in both cases (asserted end to end over TLS).

**Key order.**
- Codex, from its public source at `rust-v0.124.0` (`codex-api/src/common.rs`):
  `ResponsesApiRequest` declares `model, instructions, input, tools, tool_choice,
  parallel_tool_calls, reasoning, store, stream, include, service_tier, prompt_cache_key, text,
  client_metadata`; the websocket `ResponseCreateWsRequest` adds `previous_response_id` after
  `instructions` and `generate` before `client_metadata`, under a `type` tag. Both transports
  serialize through `serde_json::to_value` first (`endpoint/responses.rs:82`,
  `endpoint/responses_websocket.rs:226`). The wire order therefore equals the declaration order
  only when `serde_json/preserve_order` is enabled in the build — `codex-tui` enables it, and the
  shipped `codex` binary links it. A build without it emits keys alphabetically, which puts
  `model` **after** `input` and `instructions`. So `model` first and `stream` after `input` is
  the expected order for the installed CLI, not a guarantee. Not verified against captured
  traffic, nor for Codex releases newer than the pinned tag.
- Claude Code: **still unverified.** The repository holds no captured request, and the client's
  request construction is not public. The model gate must keep working for `model` in any
  position.

The synthetic generators (`crates/lr-proxy/tests/support/mod.rs`) write Responses bodies in the
Codex declaration order and can move `model` last.

## Step 1.2 as built

Types in `crates/lr-proxy/src/interceptor.rs`: `RequestHead<'a>`, `BodyPolicy`
(`PassThrough`, `Reject(u16, String)`, `Stream(StreamPolicy)`, `Buffered`), `StreamPolicy`,
`StreamMasking`, `BoundedCapture`, `ObservedExchange::request_capture`.
`ProxyInterceptor::body_policy`, `ReverseRecorder::body_policy` and `Firewall::body_policy` all
default to `Buffered`. Both transports treat `Stream` as `Buffered`.

- **Who passes through.** `ActiveInterceptor::body_policy` mirrors the two early returns of its
  `on_request`: a duplicate hop when secrets are not protected, and a path `wire::detect` does not
  recognize. Image paths stay `Buffered` (whether they are inspected depends on the body), and a
  `PassThrough` from the firewall for a recognized LLM path is downgraded to `Buffered`, because
  the firewall still evaluates every such request. `PassiveInterceptor` and `NoopRecorder` pass
  everything through. Websocket upgrades never consult the policy.
- **Duplicate hops** cannot occur on the wire today: `stamp_trace` resets every inbound hop claim
  to 1. That pass-through case is implemented and unit-tested, and unreachable until a trusted
  hand-off exists.
- **Header-time rejects** in `AppFirewall::body_policy`: unknown client, disabled client or one
  outside a proxy mode, missing strategy, an undecodable `Content-Encoding` while a model or
  GuardRails restriction needs the body (415), LLM access Off, and an exceeded rate limit. Two
  cases the buffered path does not refuse are deliberately left `Buffered`: deleting or cancelling
  a stored response while access is Off, and any retrieval while rate limited. A request refused
  for two reasons can get a different status than before (a rate-limited request for a model that
  is not allowed answers 429, not 403); it is refused either way.
- **A refused upload is drained**, not reset: the body is read and discarded (bounded head kept
  for the monitor, at most the transport's request limit read) so the client can read the
  refusal, which clients such as Codex show verbatim. A client that sent `Expect: 100-continue`
  is answered at once.
- **`on_request` is still called for pass-through**, with `request_body: None` and
  `request_capture: Some(empty)`, so path-only side effects (the unsupported-endpoint warning)
  keep working. Only `Forward` and `Reject` are honored; a rewrite answers 500 and nothing is
  forwarded. If the policy changed between the two calls so that the firewall would now judge the
  request, `ActiveInterceptor` answers 503 instead of evaluating a body it never read.
- **Monitor.** Content is captured only on recognized LLM paths (head ≤ 256 KiB, `RAW_CAP`);
  other paths record the forwarded length as `bytes_sent` on their passthrough event and no
  content. The pending event of a pass-through LLM request opens without a request half;
  `PassiveInterceptor::complete` fills it in (and correlates the conversation) from the capture
  when it holds the whole identity-encoded body. A truncated capture is shown as raw text with
  the truncation marker and is not parsed; a content-encoded one is described by length only.
  Decoding and metadata for those arrive with steps 1.4 and 1.9. Until then a passive-mode LLM
  request above 256 KiB (or content-encoded) shows no requested model, message count or tools,
  and is not correlated into a conversation; the response half is recorded as before.
- **Limits.** Pass-through has no size limit on either proxy. Buffered requests keep theirs
  (256 MiB MITM, 16 MiB reverse proxy).

## Steps 1.5–1.10 as built (2026-11-22)

### Design change: unit-granular streaming instead of byte windows
The pipeline does not mask string bytes in rolling windows (§3.4 "Masking (tail)"). It cuts the
request into **units**: every member of the top-level object, except that the `messages` and
`input` arrays are cut into their elements. Detection, remembered redactions, vault masking,
attachments, the contract and correlation all run on one unit at a time, with the functions the
buffered path runs on the whole document (`extract_request_text`, `scan_projection`,
`redact_text_files`/`protect_text_files`, the vault walk). Why:
- Parity by construction: no second implementation of `visit_with_mode`, `is_protocol_object`,
  nested-JSON decoding, signed/opaque handling or `extract_request_text` joins. The only new
  walk code is `visit_member`, factored out of `visit_with_mode` and used by both paths.
- No window cuts: every text is scanned whole, so there is no rule-span problem. The rule-span
  audit of step 1.6 (open question 3) is therefore not needed for correctness; the equivalent
  bound is the unit size (below).
- Signature-last, `role`-last, `encrypted_content`, protocol identities, nested JSON strings
  and text-file parts are all decided inside one unit's `Value`, exactly as in batch.

Cost: a unit is held (and parsed) whole. A unit larger than everything else holds the stream
behind it (fail toward hold); the hold is capped at 256 MiB like the buffered path, so a body
that is mostly one giant element behaves as today (413 above the cap), never worse.

### Release rule (`crates/lr-proxy/src/request_stream.rs`)
- Head: the lexer tracks unit boundaries (`Structure::observe`) and sniffs metadata; every
  complete unit is parsed and handed to `StreamSession::inspect` (detect + register).
- Tail: a unit is released only when `unit.end + lag <= frontier`, the frontier being the start
  of the first unit still being lexed (so an incomplete unit within reach blocks release).
  Released units are parsed again and handed to `StreamSession::mask`; unchanged units are
  forwarded byte-for-byte, changed ones re-serialized. Whitespace between top-level members is
  normalized; values keep their spelling.
- Nothing is released before `ready_to_open`: more than `lag` bytes arrived (so **a body of at
  most `lag` bytes is never streamed and takes the buffered path unchanged**), the root is an
  object with a string `model` and a `messages`/`input`/`prompt` member (fixing
  `is_protocol_object`/`request_input` for every unit). `model` may come in any position; the
  stream opens once it has arrived.
- Give up before release (→ buffered path, today's behaviour): not an object, duplicate
  top-level key, a top-level `commands`/`questions`/`state`/`signature`/`thought_signature`
  (detection or the walk needs the whole document), an over-long key, invalid JSON, a decoding
  error, the hold cap. After release the same conditions abort the upload (400/413).
- Root-level `signature` is treated as whole-document (batch would skip masking entirely).

### Abort and cancellation
The upstream body (`UpstreamBody`) is fed through a channel and ends only on an explicit end;
a dropped sender yields a body error, so hyper closes the connection without the final chunk
(transformed bodies are always chunked). The driver task (`drive`) aborts on decode errors,
pipeline problems, masking failures and client body errors; the transport answers the client
with the abort's status when the upstream had not answered (`StreamProgress::abort`). A client
disconnect fails the client body → abort; the existing `CancellationGuard` closes the pending
event. Work runs in `spawn_blocking` steps of 256 KiB.

### Detection (step 1.6)
`lr_server::routes::pipeline::stream_scan`: `SecretScanPolicy` is the policy context
(action, bypass, scanner, remembered digests) now also used by
`scan_request_for_secrets_fresh`; `StreamSecretScan::inspect` runs projection + extraction +
`scan_ranked` per unit; `FindingAccumulator` (lr-secret-scanner) reproduces batch
deduplication across calls; Redact findings are registered immediately; one aggregated
`record_secret_scan` at EOF; Notify: one popup with `request_json: None`, `request_match:
None` and ±64 KiB context. The popup UI already tolerates `request_json: null`
(`SecretScanEvent.tsx` falls back to `finding.context` and hides "View full event"), so no UI
or payload type change was needed.

Known differences for bodies above the lag (documented, not security-relevant): finding order
and first location may differ when the same value occurs under keys that the batch extractor
visits in a different order than document order; the monitor fallback preview holds 4 KiB.

### Masking (step 1.7)
`launcher/proxy_stream.rs::AppStreamSession`: remembered-redaction plan (attachments first) →
`protect_text_files` → `Vault::protect_stream_units` (opaque bindings, exemptions, signed and
identity diagnostics, the same walk) → `extensions.localrouter.session` removal (member dropped
when empty) → contract capture (`tools`, `response_format`, `text`, `input[type=additional_tools]`,
8 MiB budget, over it the existing "contract unavailable" diagnostic). A vault failure aborts the
stream (the buffered path would forward the original). Restore-only vaults (Off/Notify with a
continuation) only capture the contract.

"Earlier occurrence" diagnostic: a 16 Mibit Bloom filter of released tokens (≥ 8 bytes);
a newly registered secret matching it emits `EARLIER_OCCURRENCE`.

### Wiring (step 1.8)
`AppFirewall::body_policy` returns `Stream` for POSTs of Messages/Chat/Responses with a
decodable Content-Encoding when Secret Scan is Off/Notify/Redact and GuardRails would skip;
Secret Scan Ask stays `Buffered`. `needs_model` is set for restricted models or model Ask
(informational: the stream never opens before `model` is known anyway). The reverse proxy and
websockets keep buffering `Stream`.
`AppFirewall::open_stream` (called once the body outgrew the lag and `model` is known) repeats
`evaluate`'s content-free decisions: client/strategy, access Off (403), allowed models (403),
rate limits (429), GuardRails or a changed masking mode (→ buffered), model Ask without a time
approval or explicit allow (→ buffered, popup as today). Off without a vault forwards unchanged.

### Monitor (step 1.9)
`ObservedExchange::request_stream` (sniffed `RequestMeta`, tool count, conversation id,
correlation parts) and `request_capture` (head of the forwarded, masked bytes).
`lr_monitor::CorrelationParts`/`MessageHistory` collect correlation per element; the batch
`Correlation::from_body` now goes through the same parts, so hashes are byte-identical by
construction. Passive parses a buffered request once per event (`complete` reuses the pending
event's metadata) instead of three to four times.

### Open questions answered
- Q3 (rule spans): not needed by this design; every text is scanned whole within its unit.
- Q4 (popup with `request_json: None`): tolerated, see Detection.

### Tests (step 1.10)
- `request_stream_tests.rs`: every-byte chunking (byte-identical output for compact input),
  ≤ L never opens, late detection within/below the lag (+ diagnostic), split secrets, incomplete
  unit blocks release, tail stays L behind, give-up and abort cases, dropped members, hold cap,
  correlation parity, fingerprints, memory bound (500 MiB body: max held 17.2 MB for L = 16 MiB).
- `launcher/proxy_stream_tests.rs`: real scanner + vault parity against `evaluate` at L ≥ body
  (forced open at EOF) and at 4 KiB/16 KiB with 1-, 333- and 4096-byte chunks: masked Value
  equality, no detected original in the forwarded bytes, same findings, same correlation,
  references restore, contract captured; Notify forwards original bytes; open_stream decisions.
- `tests/streamed_inspection_e2e.rs`: real TLS, an upstream recording every byte (also of
  aborted bodies) and searching raw/escaped/nested/base64/percent spellings across reads.

## Phase 5 as built (2026-11-22, branch feat/native-server-body-limits)

The native server stays buffered. What changed:
- **Configurable limit** `server.max_request_body_bytes` (serde default 16 MiB, validated
  1–256 MiB, clamped at use). Read live per request (`ConfigManager::request_body_limit`), so
  the settings change ("Request Limits" card, `update_server_config { maxRequestBodyBytes }`)
  applies without a restart and every layer sees the same value. Fixed limits stay: audio
  25 MiB, image edits 50 MiB, files `MAX_FILE_BYTES + 64 KiB`, batch control 32 KiB (extractor).
- **`body_limits.rs`** replaces `RequestBodyLimitLayer` on the main, audio and image routers,
  inside `AuthLayer` (unauthenticated callers cannot hold budget): Content-Length over the limit
  → 413 unread; received bytes over it → 413; any body-read failure the middleware caused is
  answered with its structured error (`request_too_large` / `request_body_budget_exhausted`)
  whatever the extractor mapped it to. Bodies > 8 MiB reserve permits (1 KiB units) from a
  512 MiB global budget: up front from Content-Length, or as chunks arrive (chunk held, client
  back-pressured). Waits up to 3 s, then 503 + `Retry-After: 2`. The reservation lives until
  the response body ends. A body larger than the budget takes the whole budget (runs alone).
- **One copy** (N2/N3): `secret_placeholders` buffers into an exactly sized buffer
  (Content-Length, or chunks joined once), parses once, drops the bytes, and hands the value on
  through `gateway_json::parsed_request` (extension + lazy body that serializes only if a
  non-`GatewayJson` consumer reads it). `GatewayJson` takes the value, otherwise reads bytes →
  `from_slice` → drops bytes; `from_value` moves strings. The documents scan projection is
  built only when file parts exist and dropped after the scan. No `from_reader`/spill (no
  SpillBuffer on master).
- **Fail closed**: an over-budget body is refused with 413 in every mode (the budget equals the
  live transport limit, so this is what the transport would do); a body read error is refused
  (400) instead of forwarding the prefix; the "could not be encoded; original content forwarded"
  fallback is gone (only the protected value can be forwarded).
- Measured (thread-local allocation counter, `limit_tests::native_boundary_peak_memory_per_request`,
  4.2–4.5 MB bodies, peak growth / body size; before → after):
  Redact 4.21→2.16x (one chunk), 6.20→3.16x (chunked), 7.00→4.24x and 8.94→5.29x (8192 small
  turns); Notify 2.10→1.05x, 4.10→2.05x, 3.48→1.71x, 5.41→2.77x. Remaining Redact overhead is
  the scanner's text extraction and the follow-up detector's scan.

Bugs found and fixed: unscanned forward of over-budget bodies (fail-open, divergent limits);
files capped at 16 MiB by the outer transport layer although files allow 16 MiB + framing;
plain-text 413/415/400/422 from axum/tower-http instead of structured errors; compressed
bodies (`Content-Encoding`) reported as malformed JSON (now 415
`unsupported_content_encoding`); partial body forwarded on read errors; original body
forwarded when re-encoding failed; Value + raw bytes + projection held for the whole request.

Not done: audio/image multipart bodies are still fully buffered by their handlers; Redact mode
on non-inference POST routes (embeddings, …) still forwards unscanned with a warning
(pre-existing policy); budget is fixed (not configurable).

- [x] Phase 5 Plan Review · [x] Test Coverage Review · [x] Bug Hunt

## Open questions / uncertainties
1. Do api.openai.com, chatgpt.com and api.anthropic.com accept chunked request bodies on the
   authenticated path, at large sizes? The edge accepts them (step 1.1 results above).
2. What key order does Claude Code use, and does the installed Codex match its source order?
   It affects hold sizes, not correctness.
3. Do all scanner rules have a bounded match span? (step 1.6 audit)
4. Does the secret popup UI tolerate `request_json: None`?
5. Is the S1 terminal-usage loss for responses > 1 MiB real in practice? Is the R1 Ollama blob 413
   real?

## API produced by steps 1.3 and 1.4 (input to 1.5–1.9)

### 1.3 `lr_secret_scanner::request_stream` (crates/lr-secret-scanner/src/request_stream/)
- `Lexer::new()` / `with_limits(LexerLimits { max_depth: 127, max_key_bytes: 4096,
  max_scalar_bytes: 4096 })`; `push(&[u8], FnMut(&Path, Span, Event<'_>)) -> Result<(), LexError>`;
  `finish(sink)`; `bytes_fed()`, `settled()`, `path()`, `is_complete()`, `error()`. `Clone`.
  It keeps no body content; a failed lexer keeps returning its error.
- `Event`: `ObjectStart`, `ObjectEnd { len }`, `ArrayStart`, `ArrayEnd { len }`,
  `Key { text, truncated }`, `StringStart`, `StringChunk { text }`, `StringEnd`,
  `Scalar { raw, truncated }`, `Opaque { last }` (a container nested deeper than `max_depth`:
  raw spans only, no inner events).
- `Span { start, end }` are absolute byte offsets. Spans are ordered and never overlap; the bytes
  between two spans are only whitespace, `,` and `:`. A `StringChunk` is either verbatim text
  (`text.len() == span.len()`) or exactly one escape sequence (a surrogate pair counts as one).
  Two lexers over the same bytes report the same events and spans whatever the chunking, except
  that runs of verbatim `StringChunk`s and `Opaque` pieces may be cut at different points, so
  the tail cursor can copy `held[a..b]` by offset. `settled()` is the offset below which every byte is already
  covered by a delivered event (a pending key, scalar, escape or split UTF-8 character starts
  there), i.e. what the tail may release or copy after a `push`.
- `Path`: `depth()`, `segment(i)`, `last()`, `segments()`, `pointer()` (RFC 6901),
  `matches(&[Step])`, `starts_with(&[Step])` with `Step::{Key, Index, AnyIndex, AnyKey}`. A
  container's start/end events carry the container's own path; `Key` carries the member's path.
- `LexError::{InvalidUtf8, Syntax, Truncated} { offset }`. Accepts what `serde_json` parses into
  a `Value`, except: nesting beyond `max_depth` becomes `Opaque` instead of an error, and number
  magnitude is not checked.
- `MetadataSniffer::new()` / `with_limit(bytes)`; `observe(&Path, &Event)`; `metadata()` at any
  time; `sniff(&[u8])` for a held body. `RequestMetadata`: `model`, `stream`, `kind` (`type`),
  `previous_response_id`, `client_metadata` and `extensions` (compact JSON text),
  `metadata_user_id`, `metadata_session_id` as `Sniffed::{Absent, Pending, Value, Mismatch,
  Overflow}`; `messages` / `input` / `tools` as `Option<Shape>`; `duplicate_keys`; `complete`.
  Each capture is bounded at 64 KiB. **A repeated key replaces the earlier value** (as in batch
  parsing) and sets `duplicate_keys`: a model gate that opened on the first `model` must re-check
  or fail closed when the flag is set.
- lr-proxy `wire::request_meta_from_sniffed(format, &RequestMetadata) -> Option<RequestMeta>`
  (Anthropic, Chat, Responses; parity-tested against `wire::parse_request`) and
  `wire::conversation_id_from_sniffed`.
- `json_stream::JsonStream` (response side) is unchanged and not rebuilt on the lexer: it
  interleaves restoration with lexing and its lenient passthrough-on-error contract differs.

### 1.4 `lr_proxy::request_body::StreamDecoder`
- `StreamDecoder::new(&HeaderMap)` / `with_limits(&HeaderMap, DecodeLimits { max_decoded: 1 GiB,
  max_ratio: 1024, ratio_floor: 64 MiB })` → `Err(Unsupported)` for the same headers the batch
  path rejects. `push(&[u8], FnMut(&[u8]) -> ControlFlow<()>)` delivers decoded pieces of at most
  `DECODED_BLOCK` (64 KiB); `finish(self) -> Result<u64, _>`; `is_identity()`, `encoded_bytes()`,
  `decoded_bytes()`. Synchronous and `Send` (use `spawn_blocking` for large chunks).
- `StreamDecodeError::{Unsupported, Corrupt, Truncated, BudgetExceeded, ExpansionRatio, Aborted}`
  with `status()`, `message()` and `as_batch() -> Option<DecodeError>`.
- The byte budget applies to every stage of a stack and to the encoded input. The ratio guard
  compares each stage's output with the encoded bytes received so far and only above
  `ratio_floor`; it is the one place where streaming rejects a body the batch path accepts.
- Bytes after the end of a brotli stream are ignored, because the batch decoder ignores them.
- Fixed in the batch path: `deflate` bodies decoding to more than 8 KiB were rejected as invalid
  (a first `FlushDecompress::Finish` call is one-shot in miniz_oxide).
