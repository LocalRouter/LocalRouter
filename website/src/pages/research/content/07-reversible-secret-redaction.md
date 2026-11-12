<!-- @entry redaction-abstract -->

Reversible secret redaction separates a credential's bytes from the model's ability to refer to it. LocalRouter detects selected secrets before model dispatch, stores their originals in the operating system's credential store when available, with a client-scoped memory fallback during outages, and substitutes self-describing references. Returned references are restored locally in response text and tool arguments. A tool result containing the original is protected again before the next model call.

The mechanism adds no system or developer instruction. Its reuse instruction travels in the reference itself. References remain stable for the same authenticated client to preserve identical historical prompt prefixes.

<!-- @entry redaction-threat-model -->

An assistant may need to write a configuration file containing a credential without sending that credential to a model provider. Removing the credential permanently prevents the assistant from completing the file. Sending an opaque reference lets it describe where the value belongs while the router supplies the original locally.

The protected boundary is the provider-facing model request. An authorized caller or tool can receive the original after restoration. A reference is a capability to request use of a secret within its authenticated client; it is not permission to execute a tool. Prompt injection can attempt to misuse that capability. Existing tool and destination approvals remain necessary.

Detection is heuristic. An unknown secret that does not match a detector is outside coverage. Protection is best effort: internal failures and unsupported content can pass original bytes to the provider, with local diagnostics instead of request rejection. OS credential storage does not defend against a compromised LocalRouter process or operating system.

<!-- @entry redaction-format -->

The emitted form is:

```xml
<localrouter-redacted id='abc123' usage='Unmasked when returned verbatim'/>
```

`abc123` is a shortened illustrative ID. Production IDs contain 128 random bits encoded as 32 lowercase hexadecimal characters, with collisions checked after case normalization. They do not contain a secret prefix, suffix or digest. The same original reuses its reference for that authenticated client. Different clients use unrelated IDs, even when their original values match.

The `usage` attribute explains reuse without changing the request's system instructions. Its presence is not required for recognition.

<!-- @entry redaction-recognition -->

The recognizer searches for the ASCII case-insensitive name `localrouter-redacted`, verifies a literal `<` immediately before it and an exact name boundary, then finds the closing unquoted `>`. An optional self-closing slash is accepted. Attribute parsing is quote-aware: a `>` inside `usage` cannot end the reference prematurely.

The `id` attribute name and ID value are case-insensitive. Attribute order, additional attributes, single or double quotes, bare alphanumeric IDs, and omission of `usage` are tolerated. Duplicate IDs, partial references and malformed attributes cannot authorize restoration. Lookup remains exact after normalization and confined to the request's scope.

```xml
<LOCALROUTER-REDACTED ID="ABC123">
<localrouter-redacted extra="ignored > text" Id='AbC123' />
```

These resolve the same illustrative ID. Misspelled tag names and XML/HTML entity disguises are not repaired. The scanner is bounded by the gateway input budget, does not use XML entities or DTDs, and never rescans inserted originals.

<!-- @entry redaction-storage -->

Release builds store original values in macOS Keychain, Windows Credential Manager, or the configured Linux OS credential backend. Debug builds default to a separate development file to avoid repeated Keychain dialogs; opting into native debug storage uses an isolated service per environment. Ordinary unit tests use an in-memory mock. Native tests create isolated synthetic credentials rather than using production credentials.

Mappings belong to the authenticated client and persist across LocalRouter restarts for fourteen days of client inactivity. Conversation identity is used only for Monitor grouping. No session header or provider response ID is required to recover a client's references. Existing owned conversation records migrate while preserving their reference IDs; records from different clients never merge. Expired or revoked mappings cannot resolve. A native storage failure switches to client-scoped memory while a background worker retries noninteractively, with exponential backoff capped at thirty-two seconds. Initialization errors are retryable too. Recovery merges unread saved mappings and owner registries with pending changes, retains explicit deletion tombstones, and places a revocation fence around index changes until pending values and purges are durable. Requests can keep using the memory map while recovery I/O runs. An outage event retains the operation and underlying error once; a recovery event appears after all pending state is durable. Corrupt or conflicting unread records remain preserved. Memory-only mappings do not survive an app exit before recovery. Internal scanning, masking and restoration failures never reject an LLM call; incomplete coverage produces a local diagnostic and original content can pass through. Large values use the native credential chunking layer.

Turning scanning Off or Notify stops automatic outbound masking while available owned references continue to resolve. Response retrieval, input-item listing, cancellation and deletion preserve their owned restoration context. Cancellation can still reach the provider if credential storage becomes unavailable. Fresh Off requests do not require a working native secret store.

Native Codex compaction and encrypted reasoning state remain byte-for-byte intact. Opaque provider state never grants access to another client’s mappings. Restoring references requires the same authenticated client, including after restart. Visible reasoning summaries are restored separately and protected again on replay.

Transient plaintext is needed when receiving the original, storing it and restoring a reply. Monitor retains requests, responses, errors and conversation metadata. Values selected for redaction appear as references; Notify and allowed findings retain their original local content. Each scan records its per-finding outcome, replacement and focused context. Originals can be revealed locally from an unexpired client mapping. Tokenized response histories remain tokenized at rest.

The Secret Scan store UI lists masked values, reference IDs and client activity/expiry without refreshing retention. Explicit reveal is local administration; purge can target one original and its legacy aliases, one client, or every client. Purge revokes affected IDs before native writes/deletes, preserves activity timestamps for remaining mappings, and records value-specific revocations so older requests cannot recreate purged originals. Unrelated in-flight references remain usable. A whole-client purge advances the owner epoch. A failed native removal remains revoked and available for retry. New requests may issue fresh IDs.

Monitor records **Secret Redacted** and **Secret Restored** under the same Secret Scan category, linked to the parent request/conversation. Request-local observers aggregate substitutions by reference and location, including known originals in later tool-result requests. Notifications contain metadata rather than original credential bytes and run after credential locks are released. Schema-contract validation does not create restoration events.

<!-- @entry redaction-caching -->

A fixed self-describing tag avoids injecting a new instruction or per-turn manifest. Canonical spelling, attribute order and reference IDs remain stable for the same secret and authenticated client. Model-visible history can therefore retain identical bytes across repeated requests and retries.

This establishes prefix stability, not a guaranteed provider cache hit. Provider-specific caches have additional policies. Reference reuse is client-wide and does not depend on recognizing a conversation. Activity refreshes the fourteen-day retention period; restarting LocalRouter alone does not.

<!-- @entry redaction-streaming -->

Restoration applies to model-authored response content, including ordinary prose, code, structured output and tool arguments. Wire JSON is decoded first so quotes and backslashes in an original cannot corrupt the response envelope. Protocol identities and signed or opaque provider state are preserved.

Streaming text holds a possible partial reference until its closing bracket arrives. Each choice, content block and tool call has separate state. JSON string escapes, tool arguments and structured output are restored incrementally; the adapter does not wait for the complete argument document or response. Quoted fragments retain at most 64 KiB of source context. Unchanged JSON strings retain their escape spellings, and signed provider fields are preserved. Terminal snapshots are restored separately and do not duplicate streamed deltas.

Response framing is recognized from the wire prefix as well as the media type, so missing or incorrect SSE content types do not turn streaming into whole-response buffering. Request scanning operates on individual extracted values without copying the entire history for filtering. Large multiline values use overlapping source windows, with commits outside quoted strings and matched credentials; identical decoder views are reused. Requests still complete JSON parsing and configured permission or approval checks before provider dispatch; streaming must not bypass a Deny decision. Ordinary marker examples are not reported as missing credentials. Interrupted downstream delivery is recorded separately from Secret Scan restoration diagnostics.

Unknown, malformed, or unsupported references remain literal in both prose and executable content. A metadata-only **Secret Not Restored** diagnostic reports incomplete restoration without aborting the response or guessing a credential. Storage failures fall back to client-scoped process memory. If masking cannot complete, the request continues with a local diagnostic; only configured permission and approval decisions can deny it.

<!-- @entry redaction-tools -->

For a client-executed file-write call, the client receives restored arguments:

```text
Model:  write_file(.env, "API_KEY=<localrouter-redacted .../>")
Caller: write_file(.env, "API_KEY=demo-only-secret")
```

The example is synthetic. For automatic MCP execution, LocalRouter restores an execution-only argument copy before its existing tool permissions and approval boundary. User edits are restored and schema-validated before execution, with effective permissions checked again. Assistant history remains tokenized, and known originals in tool results are replaced again before model dispatch.

Hosted provider tools execute before LocalRouter can intercept their arguments and cannot use this local restoration mechanism.

<!-- @entry redaction-evaluation -->

The deterministic test corpus exercises accepted and rejected tag variants, case-insensitive lookup, client isolation, fourteen-day inactivity expiry, restart recovery, overlapping secrets, Unicode/JSON escaping, native-store failures and one-pass substitution. Streaming cases split references at every boundary and interleave independent channels. A file-write integration verifies the actual written bytes and that a tool echo is masked on the next model request. Coverage includes Responses Lite tool declarations, custom-call input, typed tool outputs, batched prompts and JSON-escaped multiline credentials in message text and decoded attachments.

Reproduce core checks without a paid model call:

```sh
rustup run stable cargo test -p lr-secret-scanner placeholders
rustup run stable cargo test -p lr-server secret_placeholders
rustup run stable cargo test -p lr-proxy
```

An explicitly ignored native-store smoke test creates a dedicated synthetic credential, recovers it in a fresh child process and a new vault instance and removes it:

```sh
rustup run stable cargo test -p lr-secret-scanner native_os_store_round_trip -- --ignored
```

Full conversation regressions run Codex CLI 0.166.0 with its default Responses Lite and code-mode tools through the real firewall and TLS proxy to a controlled local provider. They cover HTTP/SSE and WebSocket, Redact from the start, enabling Redact mid-conversation, and turning scanning Off after references have been issued. The client executes real shell tools; tests check exact restored file bytes, subsequent tool-result requests, restored assistant text, stable references and a saved conversation resumed in a fresh Codex process. The hosted validation gate uses a mock credential store on headless Linux. The desktop fixture can use isolated native Keychain records or a mock; local debug application testing uses its separate development file. The headless gate uses Ubuntu 22.04, matching the Linux release baseline, with the normal client workspace sandbox. A runtime preflight checks that writing inside the workspace succeeds and writing outside it is denied before the Rust suite builds.

Run the pinned real-client suite with an installed CLI and native storage:

```sh
LOCALROUTER_TEST_CODEX=/path/to/codex LOCALROUTER_TEST_KEYCHAIN=native \
  rustup run stable bash scripts/test-codex-secret-lifecycle.sh
```

Additional transport regressions use the upstream Codex request encoder, chunked HTTP framing, gzip, deflate, Brotli, layered encodings, corrupt or truncated bodies, expansion limits, unchanged passive forwarding and representation headers. The reverse proxy has native Ollama NDJSON coverage. The MCP execution proof writes the restored value, rechecks edited arguments and effective permissions, and checks the tokenized response index. The native Keychain proof recovers a scope in a new process. Parser cases include adversarial input and exhaustive split positions. The landing diagram, research navigation, action selectors and Ask/Notify popup flows have browser regressions. These checks do not measure a hosted model's willingness to copy references.

The unit suite uses a mock credential store. Native platform checks and build results should be reported separately; success on macOS is not evidence that Windows or Linux native integration has been exercised. No hosted-model success rate or cache-hit benchmark is claimed here.

<!-- @entry redaction-limits -->

Secret detection is not universal and cannot inspect opaque image, audio or binary content without a separate extraction path. JSON/code-string quoting, Unicode escapes, HTML named/numeric entities, URL percent encoding, and bounded combinations of those encodings are decoded locally and reapplied to the original during restoration. Both marker names are recognized. Unmatched forms are preserved rather than guessed; arbitrary encodings are not guaranteed. Unrelated schemas and custom grammars do not reject a request merely because secret scanning is enabled. Hosted-tool declarations can coexist with redaction: provider-side execution receives masked inputs, and originals are restored only on the local return. Restored response values receive bounded local schema checks; unsupported schemas or validation issues produce diagnostics rather than aborting provider responses. Schema checks never fetch remote resources. Actual MCP execution retains its existing permission and input-validation boundary.

The text gateway and active native proxy share response restoration. The proxy covers JSON and SSE responses, Codex Responses over WebSocket and native compaction, and native Ollama NDJSON through the reverse proxy. HTTP request content encoding is decoded before policy inspection. zstd, gzip, HTTP deflate and Brotli are supported with at most four encoding layers and a 16 MiB limit on each decoded layer. Rewritten JSON uses identity bytes with stale encoding, length and checksum headers removed; upstream responses are requested with explicit identity encoding. Provider framing is decoded before restoration; masked traffic reaches monitoring before any originals return to the caller. Binary and opaque provider execution remain outside this textual mechanism. Tool execution can reveal the credential to an authorized destination even though the model provider does not receive its bytes. A caller that receives a restored answer is responsible for its own logs and subsequent handling.

Secret Scan is best effort at every model and tool boundary. Unsupported formats, signed blocks, work limits, schema setup failures and incomplete provider streams retain their original content. The only Secret Scan denial is an explicit user Deny, reported as `secret_scan_denied_by_user`; cancellation, dismissal, timeout or an unavailable approval UI proceeds with best-effort redaction. Existing authentication, firewall permissions and ordinary protocol validation remain independent.


Detection uses a pinned, offline snapshot of the MIT-licensed [Gitleaks v8.30.1 content rules](https://github.com/gitleaks/gitleaks/tree/83d9cd684c87d95d656c1458ef04895a7f1cbd8e), adapted to Rust's regex engine with upstream capture, entropy and content-allowlist semantics. LocalRouter retains format-specific validation and false-positive filters; file-path-dependent rules and public identifiers are explicitly excluded. The repository includes the upstream license, checksums, reproducible importer and regression tests. The Go scanner and network credential-verification services are not used.

An explicit **Not a secret** decision persists as a client-specific salted digest, independent of original retention. Future masking excludes that value, while all previously issued references remain available for restoration until their normal expiry or purge. Removing the remembered decision resumes masking. This separates an allow decision from destructive removal of a restoration capability.
