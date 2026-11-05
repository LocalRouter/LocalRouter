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

Original values use the existing native credential abstraction: macOS Keychain, Windows Credential Manager, or the configured Linux OS credential backend. Reversible originals do not use the development plaintext-file keychain fallback. Tests inject an in-memory mock rather than using production credentials.

An owner/session record separates clients and conversations. Requests receive only the references authorized by their protected context. Durable scopes can be recovered after a restart from OS-stored records; missing or revoked records do not fall back to another scope. A failed native write blocks dispatch rather than sending plaintext. Large values use the existing native credential chunking layer.

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

The example is synthetic. For automatic MCP execution, LocalRouter restores an execution-only argument copy before its existing tool permissions and approval boundary. User edits must be checked against the arguments actually executed. Assistant history remains tokenized, and known originals in tool results are replaced again before model dispatch.

Hosted provider tools execute before LocalRouter can intercept their arguments and cannot use this local restoration mechanism.

<!-- @entry redaction-evaluation -->

The deterministic test corpus exercises accepted and rejected tag variants, case-insensitive lookup, scope isolation, restart recovery, overlapping secrets, Unicode/JSON escaping, native-store failures and one-pass substitution. Streaming cases split references at every boundary and interleave independent channels. A synthetic file-write integration verifies the actual written bytes and that a tool echo is masked on the next model request.

Reproduce core checks without a paid model call:

```sh
rustup run stable cargo test -p lr-secret-scanner placeholders
rustup run stable cargo test -p lr-server secret_placeholders
```

An explicitly ignored native-store smoke test creates a dedicated synthetic credential, recovers it through a new vault instance and removes it:

```sh
rustup run stable cargo test -p lr-secret-scanner native_os_store_round_trip -- --ignored
```

Local evaluation on macOS with Rust stable 1.99.1 includes 83 passing scanner tests, ten gateway/boundary tests, a separate successful native Keychain smoke test, a 2,000-input adversarial parser corpus and exhaustive split positions for representative references. The gateway proof checks that model-facing input contains a reference and unchanged system instructions, while a synthetic file-write receives the original bytes. The desktop/mobile landing diagram, research navigation and Ask placeholder choice have browser regressions. These are deterministic local checks, not evidence of a particular hosted model's willingness to copy references.

The unit suite uses a mock credential store. Native platform checks and build results should be reported separately; success on macOS is not evidence that Windows or Linux native integration has been exercised. No hosted-model success rate or cache-hit benchmark is claimed here.

<!-- @entry redaction-limits -->

Secret detection is not universal and cannot inspect opaque image, audio or binary content without a separate extraction path. Arbitrary HTML/entity/URL-encoded references are not decoded as an attempt to guess the model's intended value. Strict output schemas can be incompatible with the placeholder or restored original and require explicit validation.

The protected text gateway is distinct from passive HTTPS inspection or opaque WebSocket traffic. Unsupported paths must not claim reversible protection. Tool execution can reveal the credential to an authorized destination even though the model provider does not receive its bytes. A caller that receives a restored answer is responsible for its own logs and subsequent handling.
