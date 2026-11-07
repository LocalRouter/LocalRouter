<!-- @entry redaction-abstract -->

Reversible secret redaction separates a credential's bytes from the model's ability to refer to it. LocalRouter detects selected secrets before model dispatch, stores their originals in the operating system's credential store, and substitutes self-describing references. Returned references are restored locally in response text and tool arguments. A tool result containing the original is protected again before the next model call.

The mechanism adds no system or developer instruction. Its reuse instruction travels in the reference itself. References remain stable within an explicit conversation to preserve identical historical prompt prefixes.

<!-- @entry redaction-threat-model -->

An assistant may need to write a configuration file containing a credential without sending that credential to a model provider. Removing the credential permanently prevents the assistant from completing the file. Sending an opaque reference lets it describe where the value belongs while the router supplies the original locally.

The protected boundary is the provider-facing model request. An authorized caller or tool can receive the original after restoration. A reference is a capability to request use of a secret within its authorized conversation; it is not permission to execute a tool. Prompt injection can attempt to misuse that capability. Existing tool and destination approvals remain necessary.

Detection is heuristic. An unknown secret that does not match a detector is outside this guarantee. OS credential storage does not defend against a compromised LocalRouter process or operating system.

<!-- @entry redaction-format -->

The emitted form is:

```xml
<localrouter-redacted id="abc123" usage="Unmasked when returned verbatim"/>
```

`abc123` is a shortened illustrative ID. Production IDs contain 128 random bits encoded as 32 lowercase hexadecimal characters, with collisions checked after case normalization. They do not contain a secret prefix, suffix or digest. The same original within an explicit scope reuses its reference. Other scopes use unrelated IDs.

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

Original values use the existing native credential abstraction: macOS Keychain, Windows Credential Manager, or the configured Linux OS credential backend. Reversible originals do not use the development plaintext-file keychain fallback. Ordinary unit tests use an in-memory mock. Native tests create isolated synthetic credentials rather than using production credentials.

An owner/session record separates clients and conversations. Requests receive only the references authorized by their protected context. Provider response IDs are opaque and remain unchanged: an untracked ID, including one issued before Redact was enabled, does not block an ordinary continuation or grant access to an earlier secret. An explicit owned session or recorded response binding can recover its native scope after restart. Missing, foreign and revoked scopes never trigger a global secret search. A failed native write blocks protected dispatch rather than sending plaintext. Large values use the existing native credential chunking layer.

Turning scanning Off or Notify stops automatic outbound masking while available owned references continue to resolve. Response retrieval, input-item listing, cancellation and deletion preserve their owned restoration context. Cancellation can still reach the provider if credential storage becomes unavailable. Fresh Off requests do not require a working native secret store.

Native Codex compaction and encrypted reasoning state remain byte-for-byte intact. OS-backed metadata binds the fingerprint of returned opaque state to the references authorized in that client and conversation. Replaying that exact state can recover those references after restart; modified, foreign and revoked state grants no access. Visible reasoning summaries are restored separately and protected again on replay.

Transient plaintext is needed when receiving the original, storing it and restoring a reply. Secrets are excluded from normal monitor payloads and tokenized response histories remain tokenized at rest.

<!-- @entry redaction-caching -->

A fixed self-describing tag avoids injecting a new instruction or per-turn manifest. Canonical spelling, attribute order and reference IDs remain stable for the same secret in an explicitly identified conversation. Model-visible history can therefore retain identical bytes across repeated requests and retries.

This establishes prefix stability, not a guaranteed provider cache hit. Provider-specific caches have additional policies. Without an explicit conversation identity, LocalRouter uses a request-local scope rather than guessing a conversation or sharing secrets client-wide.

<!-- @entry redaction-streaming -->

Restoration applies to model-authored response content, including ordinary prose, code, structured output and tool arguments. Wire JSON is decoded first so quotes and backslashes in an original cannot corrupt the response envelope. Protocol identities and signed or opaque provider state are preserved.

Streaming text holds a possible partial reference until its closing bracket arrives. Each choice, content block and tool call requires separate state. JSON tool arguments are buffered through completion, parsed, restored and serialized before being returned for execution. Terminal snapshots must agree with the deltas and must not duplicate the secret.

A truncated or unknown executable reference produces a local error. Unresolved references in ordinary prose remain literal rather than guessing a credential.

<!-- @entry redaction-tools -->

For a client-executed file-write call, the client receives restored arguments:

```text
Model:  write_file(.env, "API_KEY=<localrouter-redacted .../>")
Caller: write_file(.env, "API_KEY=demo-only-secret")
```

The example is synthetic. For automatic MCP execution, LocalRouter restores an execution-only argument copy before its existing tool permissions and approval boundary. User edits are restored and schema-validated before execution, with effective permissions checked again. Assistant history remains tokenized, and known originals in tool results are replaced again before model dispatch.

Hosted provider tools execute before LocalRouter can intercept their arguments and cannot use this local restoration mechanism.

<!-- @entry redaction-evaluation -->

The deterministic test corpus exercises accepted and rejected tag variants, case-insensitive lookup, scope isolation, restart recovery, overlapping secrets, Unicode/JSON escaping, native-store failures and one-pass substitution. Streaming cases split references at every boundary and interleave independent channels. A file-write integration verifies the actual written bytes and that a tool echo is masked on the next model request. Coverage includes Responses Lite tool declarations, custom-call input, typed tool outputs, batched prompts and JSON-escaped multiline credentials in message text and decoded attachments.

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

Full conversation regressions run Codex CLI 0.166.0 with its default Responses Lite and code-mode tools through the real firewall and TLS proxy to a controlled local provider. They cover HTTP/SSE and WebSocket, Redact from the start, enabling Redact mid-conversation, and turning scanning Off after references have been issued. The client executes real shell tools; tests check exact restored file bytes, subsequent tool-result requests, restored assistant text, stable references and a saved conversation resumed in a fresh Codex process. The hosted validation gate uses a mock credential store on headless Linux. The desktop variant uses isolated native Keychain records. The headless gate uses Ubuntu 22.04, matching the Linux release baseline, with the normal client workspace sandbox. A runtime preflight checks that writing inside the workspace succeeds and writing outside it is denied before the Rust suite builds.

Run the pinned real-client suite with an installed CLI and native storage:

```sh
LOCALROUTER_TEST_CODEX=/path/to/codex LOCALROUTER_TEST_KEYCHAIN=native \
  rustup run stable bash scripts/test-codex-secret-lifecycle.sh
```

Additional transport regressions use the upstream Codex request encoder, chunked HTTP framing, gzip, deflate, Brotli, layered encodings, corrupt or truncated bodies, expansion limits, unchanged passive forwarding and representation headers. The reverse proxy has native Ollama NDJSON coverage. The MCP execution proof writes the restored value, rechecks edited arguments and effective permissions, and checks the tokenized response index. The native Keychain proof recovers a scope in a new process. Parser cases include adversarial input and exhaustive split positions. The landing diagram, research navigation, action selectors and Ask/Notify popup flows have browser regressions. These checks do not measure a hosted model's willingness to copy references.

The unit suite uses a mock credential store. Native platform checks and build results should be reported separately; success on macOS is not evidence that Windows or Linux native integration has been exercised. No hosted-model success rate or cache-hit benchmark is claimed here.

<!-- @entry redaction-limits -->

Secret detection is not universal and cannot inspect opaque image, audio or binary content without a separate extraction path. Arbitrary HTML/entity/URL-encoded references are not decoded as an attempt to guess the model's intended value. Known incompatible strict schemas and unverified constrained tool grammars are rejected before dispatch. The pinned Codex apply_patch, exec and js_repl grammars are recognized, including declarations in Responses Lite input prefixes, and namespaced function schemas stay distinct even when functions share a name. Hosted-tool declarations can coexist with redaction: provider-side execution receives masked inputs, and originals are restored only on the local return. Restored tool arguments and structured output are validated locally with bounded schemas; remote references and recursive schemas are refused.

The text gateway and active native proxy share response restoration. The proxy covers JSON and SSE responses, Codex Responses over WebSocket and native compaction, and native Ollama NDJSON through the reverse proxy. HTTP request content encoding is decoded before policy inspection. zstd, gzip, HTTP deflate and Brotli are supported with at most four encoding layers and a 16 MiB limit on each decoded layer. Rewritten JSON uses identity bytes with stale encoding, length and checksum headers removed; upstream responses are requested with explicit identity encoding. Provider framing is decoded before restoration; masked traffic reaches monitoring before any originals return to the caller. Binary and opaque provider execution remain outside this textual mechanism. Tool execution can reveal the credential to an authorized destination even though the model provider does not receive its bytes. A caller that receives a restored answer is responsible for its own logs and subsequent handling.
