# Secret reference literal handling

- [x] Inspect supplied LLM and Secret Scan events without exposing originals.
- [x] Reproduce the returned tool code failure with synthetic values.
- [x] Distinguish bare tag mentions from malformed references while rejecting unknown IDs.
- [x] Add fragmented WebSocket/tool regression coverage.
- [x] Plan Review: verify implementation against the plan.
- [x] Test Coverage Review: verify literals, malformed tags, unknown IDs, and escaped content.
- [x] Bug Hunt: reread changed code and boundary handling.
- [x] Run applicable local Rust checks.
- [x] Commit and push validated changes, preserving unrelated files.

## Supplied-event diagnosis
The outbound and returned reference IDs match. The Code Mode tool input also contains a literal `'<localrouter-redacted'` substring check. Strict restoration treats the bare tag name as a malformed reference and aborts the WebSocket response. The fix applies the parser's existing name boundary when rejecting malformed references. Unknown IDs and malformed tags remain errors in executable content.

## Additional robustness review (unfixed, outside the narrow parser repair)
Synthetic local probes in `/tmp/localrouter-robustness-review/` confirmed:
- Known originals containing quotes remain unmasked when JSON-escaped inside freeform JavaScript tool history.
- Restoring originals containing quotes into JSON embedded in JavaScript produces invalid JavaScript.
- Tool schema validation rejects unchanged calls using `$id`, and unrelated strict string constraints cause pre-dispatch rejection when any client references are active.
- Purging one entry increments the whole-client epoch and invalidates unrelated in-flight references.
- Complete literal example tags in executable source still fail as unknown references.
These findings do not involve native credential access. They require follow-up design/implementation; passing the existing workspace suite does not resolve them.

## Validation
Rust stable 1.99.1 was refreshed and used through rustup; PATH and RUSTC were set to the rustup compiler for workspace checks. `cargo test --workspace` passed 5,046 tests with 84 ignored and zero failures. `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check` passed. The final scanner suite passed 124 tests with two native-store tests intentionally ignored. No native Keychain access, paid hosted checks, deployment, or release was performed.
