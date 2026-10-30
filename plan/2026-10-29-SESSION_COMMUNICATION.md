- [x] Inspect current client identity, MCP session lifecycle, virtual servers, privacy gates, and recent conversation plans.
- [x] Check official MCP documentation and optional adapter examples for identity and delivery limitations.
- [x] Compare within-client and cross-client scopes; propose a minimal first release.
- [x] Incorporate the user's clarification: the core feature supports generic clients; app-specific integrations are optional.
- [x] Record unresolved push and native-conversation identity problems, the two-Claude-CLI case, MCP SEP-2575, and Claude Channels.
- [x] Archive this proposal as deferred; make documentation the only authorized change for now.
- [ ] Phase 0: resolve native-conversation identity and targeted push feasibility before choosing a support level.
- [ ] Phase 0b: prove portable enrollment and messaging with two generic participants, including shared/stateless transport.
- [ ] Phase 1: implement participant capabilities and bounded same-client mailboxes.
- [ ] Phase 2: expose MCP tools, permissions, and minimal desktop controls; verify two-session exchange.
- [ ] Phase 3: optionally add proven client-specific delivery adapters, such as Claude Channels.
- [ ] Future decision: consider explicit cross-client collaboration spaces after validating the within-client workflow.
- [ ] Plan Review after implementation: compare every specified behavior with the code and implement omissions.
- [ ] Test Coverage Review after implementation: cover modified paths, edge cases, and failures.
- [ ] Bug Hunt after implementation: inspect identity, authorization, lifecycle, races, bounds, and privacy.
- [ ] Run applicable local Rust, frontend, website, and native-client checks before any implementation commit.
- [ ] Commit and push completed, validated implementation and automatic catalog updates, preserving unrelated changes.
- [x] Review and validate this planning document for its separate documentation-only commit and push.

# Session communication within and across clients

Created: 2026-10-29. Updated: 2026-10-30. **Status: deferred proposal / ideas and open problems. Do not implement for now.** The user requested that the documentation be saved on master; all unchecked implementation steps above are possible future work and require a new implementation request. Initial code inspection: `c3a49e10`; latest documentation review: `c62f81a1`.

## Feasibility status and open problems

This is not a completed solution to automatic inter-conversation communication. Code and official documentation have been inspected; no two-client messaging prototype, native conversation-isolation experiment, or targeted idle-wake experiment has been run.

| Requirement | Candidate idea | What remains unresolved |
| --- | --- | --- |
| Separate explicitly registered inboxes | Server-issued participant ID and private capability | Proposed only; needs implementation and tests |
| Reliably associate an inbox with one native conversation | Host-held binding or a verified per-instance bridge | Generic tool clients expose no universal native conversation identity; copied/lost handles, forks, resumes, and resets need handling |
| Push a notification to a subscribed host | MCP SEP-2575 `subscriptions/listen` and resource-change notifications | Inbox resource support, participant authorization, subscription scoping, reconnect recovery, and host compatibility |
| Make an idle model process the message in the right conversation | Cooperating host turn API or an optional adapter such as Claude Channels | No universal MCP guarantee; each supported integration needs end-to-end proof |
| Communication across configured clients | Explicit user-created collaboration spaces | Still a future product/policy choice; same-client scope remains the first candidate |

**A mailbox protocol alone does not satisfy reliable native-conversation identity or automatic push-to-model delivery.** Future implementation must either prove those stronger guarantees for a declared support matrix or explicitly agree on a narrower pull-mailbox scope. This document does not silently choose the narrower feature on the user's behalf.

## Candidate direction if revisited

Consider **client-neutral session communication**, initially scoped to participants sharing one configured LocalRouter client. The portable mailbox candidate lets clients call communication tools and retain participant capabilities to publish a short description, discover peers, exchange direct messages, and reply. No Claude-specific mechanism is required for that logical mailbox protocol; reliable native-conversation binding and automatic delivery have the additional requirements documented here.

Defer cross-client access until the first workflow is useful. Later, add explicit collaboration spaces containing selected LocalRouter client IDs. Do not introduce a global directory or infer permission from app brand, project path, or shared machine.

There are two independent axes: **application compatibility** (generic clients from the start) and **sharing scope** (one configured client first, explicit cross-client spaces later). Supporting arbitrary apps does not authorize communication between different LocalRouter credentials. Claude Channels and similar mechanisms are optional delivery adapters on the same broker.

The product promise is: **“Participating sessions can find one another and exchange messages through LocalRouter.”** Participation is explicit protocol enrollment; universal enumeration of open app tabs and automatic interruption are not generic MCP capabilities.

## What the user experience should be

1. Enable Session Communication on a configured LocalRouter client. Explain that participating sessions using this credential share a communication scope, including sessions in different apps or projects.
2. In two conversations, ask them to participate as “Implement authentication” and “Review authentication tests.” Each calls the join tool and retains its own participant capability. A cooperating host can manage this automatically through the same protocol; optional adapters may improve enrollment and lifecycle handling.
3. The implementation session lists peers and sees the reviewer’s deliberately published name, brief task description, recent activity, and delivery capability.
4. It sends: “I changed the token-refresh interface. Please check the retry behavior.”
5. The reviewer checks its inbox, or receives an event through a supported adapter, and replies against the original message ID.
6. The sender can distinguish queued, collected, and replied messages. Neither session receives the other’s transcript, credentials, filesystem access, or tool permissions.

Enrollment can be explicit per conversation without a manual LocalRouter UI action each time: the agent can call the join tool when asked to collaborate. The generic fallback passes the server-issued capability on later tool calls, even when several conversations share a connection. Existing sessions may require configuration reload or restart to load tools; verify their actual callable catalog. Optional integration configuration stays at the approved project scope unless the user requests otherwise.

Client-neutral does not mean protocol-free. MCP is the initial tool surface. An inference-only client that cannot call tools does not automatically become a participant merely because its requests pass through LocalRouter. Existing MCP-via-LLM support can be evaluated as another tool adapter; it must use the same explicit participant capability rather than inferred history as identity.

Do not build project rooms, broadcasting, shared transcripts, automatic task delegation, spawning agents, or a workflow scheduler in v1. Communication coordinates independent sessions; file conflicts and their execution remain the sessions’ responsibility.

## Scope choice

| Choice | Benefit | Cost | Recommendation |
| --- | --- | --- | --- |
| Same configured client | Sessions in any compatible app collaborate within one existing policy boundary | Must maintain separate enrolled participants and describe delivery honestly | First release |
| Explicit cross-client spaces | Sessions with different credentials/tools/models cooperate | Adds membership, bilateral policy, provider/privacy boundaries, and revocation cases | Later, using the same broker |
| All clients discover each other | Minimal setup | Exposes unrelated sessions and bypasses existing credential boundaries | Do not offer |

A “client” here means a saved LocalRouter credential and configuration, not a software brand. Two configurations of the same app with different credentials are cross-client. Different apps sharing one credential are within one client. Separate client entries remain the way to separate trust boundaries in v1.

## Current code and protocol constraints

| Finding | Consequence | Evidence |
| --- | --- | --- |
| `Client` is persistent configuration; gateway sessions are runtime connections | Neither is inherently one user conversation | `crates/lr-config/src/types.rs`, `crates/lr-clients/src/manager.rs` |
| Legacy SSE and stateful Streamable HTTP can separate connections; headerless/stateless paths can share `client_id` | Use application-level participant capabilities across all transport modes | `crates/lr-server/src/routes/mcp.rs`, `crates/lr-mcp/src/gateway/gateway.rs`, `gateway/types.rs` |
| Monitor conversation groups are bounded diagnostic correlation and can be inferred | Never use them as messaging addresses or authorization identity | `crates/lr-monitor/src/conversation.rs`, `plan/2026-10-29-MONITOR_CONVERSATION_BEHAVIOR.md` |
| Virtual MCP servers already provide tools, instructions, per-session state, and permission hooks | Implement as a small built-in virtual server plus local broker | `crates/lr-mcp/src/gateway/virtual_server.rs` |
| Virtual tool dispatch receives client identity but no generic participant authentication | Resolve the participant capability against that client in a narrow invocation context | `crates/lr-mcp/src/gateway/gateway_tools.rs` |
| Scoped notifications already exist, but the current stdio bridge only forwards sequential HTTP requests | Existing bridges cannot be assumed to receive push events or preserve conversation identity | `gateway/types.rs`, `crates/lr-mcp/src/bridge/stdio_bridge.rs` |
| Restricted privacy profiles already reject MCP ingress | Preserve this restriction on every new adapter/broker ingress | `crates/lr-server/src/privacy.rs`, `crates/lr-config/src/client_security.rs` |

Older MCP transport sessions describe client/server interactions, not user conversations. Revision 2026-07-28 removes protocol-level HTTP sessions, reinforcing the need for an application-level participant identity. See the official [older session contract](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#session-management) and [current HTTP transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http).

MCP resource updates are application-controlled signals; they do not guarantee a model turn. Use tool-based inbox retrieval as the portable baseline. Adding virtual resources would also expand the existing tool-only virtual-server interface. See [MCP resources](https://modelcontextprotocol.io/specification/2026-07-28/server/resources).

### Concrete case: two separately launched Claude CLI sessions

Two independently launched `claude` processes normally have separate MCP client transports. With stdio, each owns a local server/bridge subprocess and its own pipes. With direct HTTP/SSE, each creates its own client transport to the same endpoint. Sharing configuration or credentials does not itself share a live connection. This follows the documented [Claude Code stdio startup and shutdown behavior](https://code.claude.com/docs/en/mcp#how-claude-code-stops-a-stdio-server); it has not been reproduced locally as part of this plan.

LocalRouter can nevertheless share gateway state between those transports when requests fall back to `client_id`. Current stateful Streamable HTTP initialization issues separate `Mcp-Session-Id` values and keys them by client plus session. Headerless/stateless paths need separate examination; the current `StdioBridge` forwards authenticated HTTP requests without preserving the returned session header. That is a gateway/bridge identity problem, rather than evidence that Claude shares one connection. Relevant code: `crates/lr-server/src/routes/mcp.rs`, `crates/lr-mcp/src/gateway/types.rs`, and `crates/lr-mcp/src/bridge/stdio_bridge.rs`.

Per-instance bindings therefore look practical for two concurrent CLI processes. A transport still cannot establish a durable conversation identity across reconnects, `/clear`, forks, or resumes without lifecycle evidence. Do not use TCP source ports or a persistent connection alone as a conversation ID.

## Identity and lifecycle

Keep these identities distinct: authenticated `client_id`, public LocalRouter `participant_id`, secret participant capability, participant generation, and optional host/native-conversation binding. A participant is an explicitly enrolled communication identity; the core does not certify which native chat window owns it.

- `SessionJoin` without a capability creates a participant with a public opaque ID and a separate cryptographically random bearer capability. A supplied invalid or expired capability fails rather than silently creating a replacement. Scope the capability to its authenticated client and participant generation; a public directory ID never authorizes operations. Display names and task descriptions are untrusted metadata.
- Every peer operation requires current client authentication plus the participant capability, or an equivalent verified host-held binding. The server derives the sender from that proof; never accept a caller-selected `from_session_id` as identity. Another participant's public ID only selects a destination.
- In portable mode, the join result returns the capability and subsequent tool arguments carry it. This works over shared or stateless connections and requires no native session ID. Retaining the handle in the conversation is explicit client/agent protocol state, not inferred identity.
- In optional host-managed mode, the host retains and attaches the capability outside model-visible arguments/results. A per-conversation adapter can bind it to a native session. Do not use an MCP transport key or client-wide gateway session as a substitute for that binding.
- Portable mode necessarily exposes its own capability to the host and potentially its model/provider context. Never place capabilities in server-generated directory entries, peer-visible fields, or receipts; redact known capability fields from Monitor, approvals, events, errors, replay, logs, indexing, and Memory capture paths. Instruct participants never to share their capability. Arbitrary message text can itself contain caller-supplied secrets; do not promise perfect detection of those or secrecy from client-managed transcripts. Schemas may name the capability argument but never contain another participant's value.
- A lease records recent participation, not proof that an app tab remains loaded. Successful participant operations renew it; a cooperating host can refresh it while active. Show “recently active” or “waiting for inbox check.” An adapter may additionally report connectivity, without implying model attention.
- Transport reconnect is independent of participation: the same valid capability can continue an unexpired participant. Explicit leave or lease expiry revokes that capability/address. A new join creates a new generation/address, and old messages never redirect to it. A lost join response may leave an orphan until expiry.
- Generic clients cannot reliably report every close, clear, fork, reset, or compaction. If capability ownership is lost or uncertain, enroll anew; never recover an inbox from a public ID, guessed native ID, or transcript correlation. Copying a capability into a fork denotes the same participant; independent forks/subagents must join separately. Host adapters can improve these lifecycle guarantees where supported.
- Disabling the feature/client, deleting the client, configured credential expiry, or credential revocation ends participation. Every request needs valid authentication; ordinary OAuth access-token expiry rejects that request, while successful refresh can restore access to an unexpired participant. Crashes rely on lease expiry. LocalRouter restart clears participants and mailboxes in v1.

Sharing one client credential is already sharing an authentication boundary. Participant capabilities prevent accidental routing and ordinary public-ID spoofing; this feature does not promise isolation against a malicious host controlling sessions under that credential.

### Stronger candidate: cooperating-host conversation binding

For reliable native-conversation targeting, a client-neutral host contract could require the host to:

1. Maintain its conversation identity, generation, and live-instance ownership, and bind these to a LocalRouter participant.
2. Keep the participant capability outside model context and attach it to tool calls, inbox reads, and subscriptions.
3. Report lifecycle changes, including independent forks and resets. Decide how concurrent resumes of one native conversation are owned before enabling them.
4. Subscribe for targeted inbox-change events and route them to that exact native conversation.
5. Start a supported model turn when idle, or queue the message at a safe boundary while busy; arbitrary interruption is not assumed.
6. Report acceptance into the intended conversation separately from transport receipt, model acknowledgment, and reply.

One host subscription may multiplex multiple participants if every event is authorized and routed correctly. Treat events as hints: the broker's inbox, message IDs, fetch cursor, and explicit acknowledgments remain authoritative. Reconnect must refetch pending messages rather than assume notification replay.

This is an integration contract, not a capability already available in every MCP host. A model-held capability remains a possible fallback for explicit mailbox use, with the retention/copying limitations above. The host contract and its supported-client matrix remain design/prototype work.

## Proposed tools and message contract

Names below are proposals, to be aligned with existing built-in naming during implementation.

| Tool | Contract |
| --- | --- |
| `SessionJoin` | Without an existing capability, create a participant; with a valid one, update its name/purpose or renew participation. Return public ID, delivery mode, and the capability in portable mode |
| `SessionList` | Return only visible, enrolled peers in this client’s authorized scope; exclude self; paginate |
| `SessionSend` | Send plain text to one recipient, with a request idempotency key and optional `reply_to`; return message ID and queued status |
| `SessionReceive` | Fetch this participant's bounded page of messages and receipt events; accept acknowledgments for previously collected IDs; never select another participant's inbox |
| `SessionLeave` | Revoke this enrollment/address and capability and purge its pending inbox; an authenticated client can join again as a fresh participant |

The envelope contains a server message ID, trusted sender/recipient identities and generations, server time, expiry, plain-text body, and optional reply linkage. The server validates that `reply_to` belongs to an exchange visible to both participants. No attachments, remote URLs fetched by LocalRouter, transcript references, automatic execution, or structured action payloads. Plain-text code snippets are valid messages.

`SessionSend` is nonblocking with respect to the recipient. A recipient may reply later; sender tools never wait indefinitely for a model response. Immediate inbox fetch is the simplest mailbox candidate. Another idea is `SessionReceive(wait_ms)`: hold a bounded receive call until a message arrives or the wait expires. Completing that tool result can continue an already waiting agent turn, subject to host behavior. It cannot wake a finished/idle turn without a pending call. Timeouts, cancellation, background-tool behavior, and repeated timeout/model-turn costs require testing; long-polling is not a resolved substitute for push.

Use at-least-once collection until acknowledgment, rather than deleting before a tool response reaches the caller. Acknowledgment means collection, not understanding or successful work. Scope idempotency to sender generation and request key; a retry with a different payload fails. Retain bounded deduplication records for a documented retry window and return a clear result for a duplicate already expired or revoked.

Status vocabulary: **queued**, **collected** (fetched by the recipient tool/adapter), **acknowledged** (explicit recipient-agent receipt), **replied**, and **expired/revoked**. An optional channel handoff means only that the adapter wrote to its transport. Neither adapter collection nor handoff proves model consumption. A successful socket write is never “read” or “acted on.” No durable or exactly-once promise is made.

Send bounded status receipts to the sender's `SessionReceive` feed, with an explicit message/receipt event discriminator, so it can observe collection, acknowledgment, reply, and expiry after the initial queued result. Apply policy and generation checks to receipts too, avoid recursive receipts for receipts, and expire them within the same memory and retention budget. Once receipt history expires, return status unavailable rather than inventing a delivery outcome.

Use explicit text/byte/count limits from the first implementation. Initial values to validate: 4 KiB text per message, 100 queued messages or 512 KiB per participant (first limit wins), 4 MiB per client, 16 MiB globally, 15-minute message expiry, 20 sends per minute per sender, and 60 per recipient. Bound participants, directory pages, receipt history, deduplication records, and cleanup work as well. Reject a full inbox; never silently discard pending messages to accept a new one. Use a monotonic clock for runtime expiry.

## Permission and privacy behavior

- Add a default-Off per-client Session Communication setting. Reuse Off/Ask/Allow conventions. New enrollment requires enabled policy and current client authentication; updating a participant, listing, sending, and receiving also require its valid capability or host binding. Sending and receiving check current policy for both parties.
- With Ask, enrollment needs a participation approval for that logical participant; an agent-supplied name is not proof of a native conversation. The first send establishes a separate exchange grant covering the specific sender, recipient, scope, and participant generations; the approved exchange can include replies. Tool-name-only session approval is too broad. Inbox reads/receipts recheck that grant without prompting for every collection.
- Recheck current policy at enqueue, collection, and acknowledgment. Revocation removes visibility, invalidates grants, and purges undelivered content involving the revoked participant. Use the same unavailable response for hidden, expired, or unknown targets when the caller is unauthorized to distinguish them.
- Render peer messages as attributed untrusted content. They convey no system authority, approval, credentials, or access to the recipient’s tools. Prevent raw markup from impersonating host-generated envelopes. Enforce loop/rate budgets for automated replies; no default auto-reply loop or recursive delegation.
- Directory entries publish only supplied short names and purposes alongside public participant metadata. Do not derive directory titles from Monitor prompts, read transcripts, or expose roots, native IDs, capability values, model configuration, or client secrets there.
- Keep inboxes in bounded memory only. Do not automatically write messages to Memory, searchable context stores, replay payloads, or tool-result indexing. Mark all communication tools non-indexable and review generic tool-call capture paths as well.
- Monitor may record bounded IDs, sizes, timing, and outcome under current privacy settings; message bodies are omitted by default. Any later body inspection needs explicit product design and existing payload/redaction controls. Metadata-only settings must suppress body capture across desktop events, logs, errors, and demos.
- Preserve current MCP rejection for `local_only` and `no_retention` profiles; a new adapter endpoint must not become a bypass. No relaxed “RAM does not count” exception. Changing these profiles is outside this feature.
- Explain accurately: **LocalRouter relays locally, but a recipient may include the message in its next model-provider request.** LocalRouter makes no extra inference requests for discovery, titles, or delivery. There is no telemetry, external asset loading, public relay, or cross-device delivery.

## Delivery integrations

| Mode | Supported promise | Release position |
| --- | --- | --- |
| Generic MCP tools | Any capable client can join, retain its capability, discover/send/check its inbox during a turn | Candidate mailbox baseline, including shared/stateless connections |
| Generic host-managed integration | Host owns conversation binding, retains its capability, and can route events/schedule delivery | Candidate for stronger native-conversation and push guarantees |
| Client-specific push/hook adapter | Host receives an event or inbox hint through a supported mechanism | Optional way to supply stronger guarantees; unnecessary for pull-only mailboxes |

These are proposed capability levels, not claims of implemented or tested support. Generic tools can establish logical participant identity and mailbox exchange without an app-specific adapter. The stronger native-conversation and idle-wake guarantees need a cooperating host or an adapter that supplies them. The feature as originally discussed must not be declared complete solely because pull-mailbox acceptance passes.

### MCP SEP-2575: standard push-notification transport

[SEP-2575, Make MCP Stateless](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/seps/2575-stateless-mcp.md), is Final and introduces `subscriptions/listen`. It opens a long-lived notification subscription over an HTTP POST response stream or stdio. Clients opt into supported change types, including `resourceSubscriptions`; notifications carry `io.modelcontextprotocol/subscriptionId` for correlation. This is a real standard mechanism for pushing changes to a listening client. It does not introduce a universal native-conversation identifier or a model-turn scheduling API.

The subscription acknowledgment confirms establishment of the notification stream, not message collection, model processing, or a reply.

Use the current [2026-07-28 specification changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog) and [Streamable HTTP contract](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http) for normative behavior; the accepted SEP is a historical record and has subsequent corrections. Earlier revisions already provided notification mechanisms; SEP-2575 replaces their subscription/transport arrangement rather than inventing all server-to-client push.

A possible inbox mapping is a per-participant resource URI, subscribed through `resourceSubscriptions`, with `notifications/resources/updated` acting as an inbox-change hint. The host would fetch queued messages and acknowledge them through the mailbox contract. Resource URI, subscription request ID, and transport connection are routing references, not participant credentials. Authorizing resource reads/subscriptions with the same participant proof remains an explicit design problem, especially for generic hosts that only know how to pass that proof as a tool argument. Do not expose a private inbox merely because a caller knows its URI.

LocalRouter already handles `subscriptions/listen` and filters supported notification types in `crates/lr-server/src/routes/mcp.rs`. It has no communication inbox resource or per-participant notification authorization yet, and the virtual-server abstraction currently supplies tools rather than resources. Reusing that transport requires a fresh isolation audit; its presence does not establish that this feature is implemented. Extending virtual resources is a separate scope choice, so tool-based inbox retrieval remains an alternative.

### Optional example: Claude Code

Claude Code hooks expose `session_id`; returned context is consumed on a later model request. Use only lifecycle fields needed for identity. Avoid transcripts and transcript watching. Prefer a bounded “messages waiting” hint leading to an inbox tool over arbitrary message bodies inserted by hooks. Keep integration scoped to the approved project unless the user requests global configuration. See [hooks reference](https://code.claude.com/docs/en/hooks).

Claude Code Channels are a research preview using a local stdio subprocess with `claude/channel` and `notifications/claude/channel`. Custom channels currently need development opt-in or approved distribution; organization policy may prevent them. They offer a route to push, with no recipient acknowledgment guarantee. See [Channels reference](https://code.claude.com/docs/en/channels-reference).

Current Claude Code docs also say channels negotiating MCP 2026-07-28 are not registered. Prototype an earlier-protocol adapter locally; avoid changing every configured server’s protocol or enabling telemetry/flag fetching for this feature. If it cannot work under the user’s existing settings, show it as unavailable. See [MCP channel compatibility](https://code.claude.com/docs/en/mcp#push-messages-with-channels). Recheck these preview limitations at implementation time.

An adapter is a thin per-conversation bridge to the same broker, not a second messaging system. Keep app-specific adapters optional; full targeted-delivery acceptance still needs a proven host/push path unless a narrower mailbox scope is explicitly chosen. Do not repurpose sampling or resource-list notifications to imply an existing chat received a user turn. Other apps can use the generic tools without implementing an equivalent to Channels. App-specific automatic discovery/wake claims need separate proof; do not expose LocalRouter publicly to make a cloud connector reach localhost.

## Implementation shape

Reuse `lr-mcp` and its virtual-server infrastructure. Avoid a new crate, persistence layer, or generalized orchestration framework.

```mermaid
flowchart LR
  A[Participating conversation A] --> T[Communication MCP tools]
  B[Participating conversation B] --> T
  T --> C[Client authentication and participant capability]
  C --> P[Current participation and permission checks]
  P --> D[Local directory and bounded inboxes]
  D --> R[Recipient inbox retrieval]
  D -. optional .-> H[Verified client delivery adapter]
```

Resolve participant capabilities against current client authentication in a narrow virtual-call context. Store directory/mailboxes independently of gateway transport sessions so shared/stateless calls can address distinct participants. Optional host bindings can reference the same service. Do not clone mutable inboxes into per-call state. Keep session/broker locks out of network delivery and approval awaits; enforce policy versions/generations when returning from those awaits.

Likely touched areas:

- `crates/lr-mcp/src/gateway/`: communication virtual server/broker, participant capability resolution, cleanup, listing-change snapshots, instructions, and capability/payload capture exclusion.
- `crates/lr-config/`: default-Off client policy, validation, serialization/backward compatibility, and current-policy lookup.
- `crates/lr-server/` and application startup: broker wiring; authenticated adapter access only if an optional adapter is separately selected. Annotate/document any new HTTP endpoints with existing OpenAPI conventions; no redundant public REST messaging API.
- `src/views/clients/`: a Session Communication card under Client → MCP; simple participant/status view with withdrawal and administrator revocation controls. No separate global chat application or permanent message history.
- `src-tauri/src/ui/`, `src/types/tauri-commands.ts`, and `website/src/components/demo/TauriMockSetup.ts`: synchronized commands/types/demo data for the settings and participant view. Rust snake_case params, frontend camelCase, nullable optional fields, established icons, and Radix dialogs.

## Possible future phases and acceptance gates — all deferred

Do not execute these phases now. A future implementation request must revisit the unresolved requirements and choose the intended support level first.

**Phase 0 — resolve identity and delivery feasibility.** Audit two separately launched CLI processes, multiple conversations sharing one host connection, legacy stateful HTTP, stateless/headerless calls, and the stdio bridge. Record actual gateway keys alongside available native conversation IDs. Test reconnect/resume/reset/fork and distinguish logical inbox separation from native-chat binding. Prove standard subscription notification delivery separately from waking an idle model; test at least one cooperating-host or Channels path for targeted wake. If the stronger guarantees cannot be established, record the unsupported cases and revisit scope rather than calling pull-only support equivalent. No protocol or native-host experiment has been run during this planning task.

**Phase 0b — prove the portable mailbox candidate.** Enroll two participants on one credential over a shared/stateless transport; exchange a message and reply; reject public-ID-only inbox access. Exercise capability loss/copy, orphaned enrollment, leases, policy checks, and capture redaction. Test real generic tool-capable hosts and optional bounded waiting with their actual timeouts. This is baseline protocol evidence, not proof of automatic native conversation discovery or idle delivery.

**Phase 1 — broker and identity.** Implement default-Off policy, portable capability registration/resolution, generations/leases, same-client scope, direct messages, receipts, deduplication, expiry, revocation, and memory/rate bounds. Cross-client access is hard-denied. Keep the policy entry point reusable without shipping dormant cross-client configuration.

**Phase 2 — useful generic within-client experience.** Add the five tools and concise instructions for retaining capabilities, pairing-aware Ask behavior, settings, participants/status view, privacy-safe Monitor metadata, mocks/types, and real conversations exchanging a review request and reply. Verify the actual sessions loaded the tools. Explain when inbox checks are required and capability loss requires re-enrollment. Mailbox acceptance and full native-conversation/push acceptance are separate requirements.

**Phase 3 — optional host and delivery adapters.** Add host-held capability management, native lifecycle signals, or push mechanisms only where useful. Claude Code hooks/channels are one possible integration, not the defining client. Add explicit setup and capability reporting only after real tests prove it works. Preserve pending messages when notification delivery fails. Track adapter collection, transport handoff, and explicit recipient-agent acknowledgment separately. Core mailbox rollout never depends on client-specific or research-preview support.

**Later — explicit cross-client spaces.** Proceed only after a useful same-client trial. The user creates a space and selects client memberships; both clients opt in. Directory visibility, send, and receive all check membership and current policy. Include source client attribution and provider/privacy implications. Removal invalidates grants and purges pending content immediately. Project paths remain labels, not authorization. No agent can add a client to a space. Messages retain their original sender; the receiving client grants no inherited tool/model authority.

## Required validation for implementation

Test two participants on one credential and shared/stateless transport, two credentials using the same app, different apps sharing one credential, public-ID spoofing, invalid/wrong-client capabilities, lost join responses, copied capabilities, reinitialize/reconnect, stale generations, crash/lease expiry, and client credential expiry/deletion. In optional adapter tests, additionally verify native-ID collisions, reset/fork/compaction/close signals, and multiple processes resuming one conversation. Test revocation during approval, enqueue, fetch, notification, and acknowledgment; enforce a documented atomic policy boundary.

Test concurrent sends/readers, order within an inbox, duplicate/reordered retries, acknowledgment ownership, forged reply links, queue/byte/rate limits, cleanup bounds, Unicode byte limits, full queues, partial adapter failure, no listeners, expired messages, process restart, and error redaction. Verify that capabilities and bodies never enter LocalRouter Monitor/indexing/Memory/logs/replay through generic tool arguments or response previews. A recipient reply loop must remain bounded and cancellable.

Verify no tool exposure or background work when Off, live listing refresh or honest reload guidance when enabled, existing privacy ingress on every path, and no reliance on Monitor retention. LocalRouter must not initiate inference for discovery, titles, or relaying; receiving hosts may make their normal provider requests to process delivered messages. Mailbox acceptance requires an actual message and reply between two independently enrolled participants without native hooks/channels. Full native-conversation/push acceptance additionally requires two real conversations with verified separate ownership, targeted idle delivery, busy-turn queuing, reconnect recovery, and receipts. A handshake or notification write alone satisfies neither. Advertise automatic delivery only for integrations whose native acceptance passes.

For Rust/workspace changes, update and pin rustup stable, confirm actual compiler path, then run workspace Clippy with all targets and warnings denied, formatting check, and workspace tests per AGENTS.md. For desktop/shared frontend, run `npx tsc --noEmit`, `npm run test:unit`, `npm run build`, and `npm --prefix website run build:check` with dependencies installed. Run packaging-script tests if adapter packaging changes those scripts. Do not dispatch hosted CI unless requested; an ordinary master push does not imply hosted validation.

Complete the checklist’s Plan Review, Test Coverage Review, and Bug Hunt after implementation, fix findings, then commit/push validated changes with Conventional Commits and any automatic catalog update. Preserve unrelated user files. Implementation must be a separately requested task.

## Planning-delivery validation

Reviewed against repository code and official protocol/client documentation, with independent architecture, product/privacy, protocol, and generic capability reviews. The 2026-10-30 consolidation preserves client neutrality, optional cross-client scope, MCP SEP-2575, Claude Channels, the two-CLI transport distinction, candidate mailbox/host contracts, and unresolved guarantees. Implementation is explicitly deferred by the user. Document structure, all 14 listed repository references, requested-topic coverage, and whitespace checks passed; the task diff contains only this plan. SEP-2575 and Channels source links were checked against their official documentation. No Rust/frontend builds or runtime messaging experiments were run for this documentation-only task. Commit/push outcome is reported with delivery.
