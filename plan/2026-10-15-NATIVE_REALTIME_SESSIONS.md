# Native realtime sessions

- [x] Review the existing realtime proposal, installed OpenAI SDK session/events and current native OpenAI/LLM Gateway WebSocket contracts. Verify handshake/authentication, bounded settings and cancellation semantics against primary sources.
- [x] Implement a separate default-off realtime client grant, synchronized native operation types/settings/demo/discovery and monitoring labels. Require an explicit provider/model; reject strict privacy, free-only and unverified cost/token ceilings.
- [x] Add native provider WebSocket transports for public OpenAI API-key and LLM Gateway instances. Keep credentials server-side, pin configured origin/model and routing settings, and prohibit redirects, retry/reconnect and fallback.
- [x] Add authenticated bounded local realtime client tickets for browser use. Tickets expire quickly, are one-use, owner/model-bound, stored hashed in memory and never accepted outside the realtime upgrade. No credentials in URLs; normal client Bearer authentication remains supported for SDK clients.
- [x] Implement the actual WebSocket relay and native text/audio JSON events, not only ticket minting. Bound frames, bytes, duration, concurrency and response creation. Model/session controls cannot escape limits or enable hosted tools, remote references or automatic MCP execution.
- [x] Apply current client/model/credential policies before connection and throughout the session. Reject opaque blocking scanner policies rather than claiming to inspect audio. Handle grants/credentials revocation, provider changes, client disconnect, native errors, timeouts and response cancellation by closing both sides; no automatic reconnect/resubmit.
- [x] Preserve native response usage by response ID without double counting repeated snapshots. Mark unavailable fees/usage unknown rather than fabricating text/audio conversion or a zero full-session cost. Do not log audio/frame payloads.
- [x] Add API aliases, OpenAPI handshake/ticket schemas and capability discovery. Add a client-only Try It Out text event workflow with bounded event display and explicit connection/disconnection, using labeled local demo fixtures without remote assets.
- [x] Add local mock WebSocket tests for real bidirectional relay, credential isolation, owner/ticket reuse/expiry, model/session bounds, policy changes, unsupported events, usage snapshots, upstream failures, disconnect/cancellation and no retries.
- [x] Plan Review: compare all promised behavior with the actual implementation and finish gaps; update the overall gateway feature checklist only when supported end-to-end.
- [x] Test Coverage Review: cover meaningful lifecycle and error branches and regression-test existing native operations.
- [x] Bug Hunt: re-read handshake/auth, ticket races, control events, byte/duration budgets, concurrent sessions, usage merging, cleanup and payload retention; resolve defects.
- [x] Run the exact stable CI compiler/toolchain Clippy/fmt/workspace tests, frontend regressions, app/demo type checks and production builds, and a Chrome production smoke. Use the isolated validation target.
- [x] Commit and normal push validated task changes and automatic catalog updates, preserving unrelated release drafts; monitor cloud CI and resolve failures.
- [x] Complete the overall approved gateway roadmap review/coverage/bug hunt and report what shipped and concrete supported limits.

Native realtime is a WebSocket service. Browser tickets are credentials for the
local relay only, not leaked upstream client secrets. Ticket expiry limits opening
a connection; it does not claim to end an already open session. Local duration
and traffic caps must enforce the session lifetime independently.

Initial verified mappings: OpenAI public API-key realtime and LLM Gateway's
OpenAI-compatible JSON text-frame WebSocket endpoint. Audio is base64 inside JSON
frames. Do not infer that arbitrary OpenAI-compatible providers support realtime.
Selected policies must be enforced for the whole stream, and unknown provider
billing units cannot silently satisfy a configured cost/token ceiling.

Primary references: installed node_modules/openai/resources/realtime and the
current official OpenAI realtime API documentation; https://docs.llmgateway.io/features/realtime;
existing plan/2026-03-15-API-REALTIME.md, reconciled with current architecture.

Reconciled decisions (2026-10-15): the older March proposal's binary-frame
forwarding and automatic reconnection do not apply. The verified gateway accepts
JSON text frames and base64 audio only. Start with OpenAI public API keys and
LLM Gateway; do not inherit unverified Together/LocalAI claims. Local tickets
are one-use credentials for LocalRouter, not vendor secrets, with a 60-second
opening TTL. Check the original local credential fingerprint again at upgrade
and during relay so rotation/revocation also closes an established connection.

Initial session bounds: 120 seconds, 1 MiB/frame, 64 MiB traffic total,
20 explicit response creations, 1,024 output tokens per response, two active
sessions/client and sixteen globally. No automatic reconnection. Enforce manual
response creation: configure native turn detection off before accepting content,
wait for a native session.updated acknowledgement, and validate subsequent
session updates. VAD may be enabled only without automatic response creation or
idle-timeout generation. Model is pinned. Function tools execute in the client,
never LocalRouter; hosted MCP/search/tools, stored prompt references, image/file
references and additional transcription models need separate verified policy
and are outside this initial mapping. SDK clients can send native base64 audio;
Try It Out initially demonstrates text events and the actual relay lifecycle.

Usage is keyed by native response ID; repeated native usage snapshots replace,
not sum, that response's counters. Unknown audio/time fees remain unknown. A
local socket close ends the relay; it cannot promise a provider bills nothing
for already generated output. Native response.cancel is forwarded as an actual
native event while a socket is connected; cleanup closes both connections.

Implementation/review status: native provider mappings, browser tickets, actual
WebSocket text/audio relay, current-policy/credential/binding checks, bounded
manual sessions, per-response usage snapshots, discovery/OpenAPI and client UI
are implemented. Five initial live WebSocket fixtures passed; the final expanded
seven-fixture suite and complete stable parity are pending. All 72 frontend
regressions, both type checks/builds and the production Realtime browser smoke
passed. Browser validation found and fixed a missing demo grant validator entry.

Bug hunt fixed connection request accounting at handshake rather than session
end, retained native usage in session monitor snapshots (not converted into
text-token dashboard aggregates), preserved native trace correlation, rejected
custom account voice references and invalid browser subprotocol authentication,
and kept ticket minting separate from inference classification. Tests now hold a
native generation in progress until a real response.cancel event, then repeat
its usage snapshot to verify deduplication. No paid provider calls were made.

Video 7a1ea8f2 is pushed and green in cloud CI (run 38772943534).

Final validation: updated stable Rust 1.99.0, confirmed the actual rustup compiler
path and pinned PATH/RUSTC. Workspace Clippy with warnings denied, formatting
check and all 111 workspace suites passed: 4,522 passed, 69 ignored, zero failures.
Seven live native WebSocket fixtures, four policy tests, two ticket concurrency/
expiry tests and two transport tests cover this increment. All 72 frontend tests,
both type checks/production builds and the final production Chrome workflow
passed. Native protocol/schema checks use local fixtures, without paid calls.

Overall approved roadmap review, coverage review and bug hunt are complete;
all selected core and later features have an implementation and saved plan.
Native limitations are explicit in docs/REALTIME.md and docs/VIDEO_JOBS.md.

Final TLS follow-up: native Realtime now initializes TLS independently of
optional inspection-proxy startup, preserving existing crypto selection and
certificate verification. An isolated-process WSS regression covers the original
dual-backend startup failure. Updated Rust 1.99.1 exact workspace parity passed:
Clippy, fmt and 111 suites with 4,523 passed, 69 ignored, zero failures; all 72
frontend regressions also passed. Main Realtime da73b29c is green in cloud CI
(run 38779717294). See 2026-10-15-NATIVE_REALTIME_TLS_INITIALIZATION.md.
