# Preserve completed Responses tool arguments

- [x] Capture a real LocalRouter response and establish where arguments disappear.
- [x] Preserve missing argument suffixes from completed function items and responses.
- [x] Cover parallel calls, already streamed arguments and terminal-only responses.
- [x] Verify the captured completed payload with the public Goose Responses codec.
- [x] Run relevant Rust tests, formatting and Clippy; report workspace limitations.
- [x] Plan review: compare the final change with every behavior above.
- [x] Test coverage review: check completion variants and error paths.
- [x] Bug hunt: review prefix reconciliation, call identities and duplicate emission.
- [x] Commit and push: commit locally. This task explicitly forbids push, merge and restarting LocalRouter until separately approved.

LocalRouter currently emits names and IDs from `response.output_item.added` but
ignores arguments carried only by terminal events. The captured ChatGPT response
contains correct arguments in `response.completed.response.output`. The fix belongs
in LocalRouter's existing Responses-to-Chat translation. Jazyk keeps its standard
GDK Chat codec and does not interpret LocalRouter-specific envelopes.

The translator reads final arguments from `function_call_arguments.done`,
`output_item.done` and `response.completed`. It emits only the missing suffix,
keeps each call's ID and index, and rejects contradictory final arguments or
identities. A completed event still produces one native envelope.

Review covered partial and terminal-only calls, parallel calls, omitted item IDs,
arguments before headers, Unicode across fragmented CRLF frames, existing custom
tool input translation and explicit error paths. No transport or parser was
replaced. The model catalog snapshot refreshed automatically during validation.

The captured real response passed both offline checks:
- The public Goose Responses codec reads two calls with the exact requested refs.
- The patched translator's Chat chunks pass the unmodified public Goose Chat
  streaming codec and collector with the same two refs.

No credentials or live response payloads are committed. LocalRouter remains
running its existing version until separate approval to merge and restart.

Validation used Rust stable `1.99.1` from rustup:
- `cargo test --workspace`: 4,690 passed, 69 ignored, zero failures across 111 suites.
- Final `cargo test -p lr-providers openai_responses::stream::tests --lib`: all
  15 passed, including the three additional review regressions.
- Final `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

The existing `block 0.1.6` dependency emits a future-incompatibility notice. No
frontend source changed. The automatic catalog snapshot now contains 8,922 models.
