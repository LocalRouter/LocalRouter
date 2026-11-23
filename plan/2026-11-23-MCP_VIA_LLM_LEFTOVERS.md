# MCP-via-LLM leftovers (chat / Responses)

Date: 2026-11-23
Branch: `fix/mcp-via-llm-leftovers`

The "Not fixed (reported)" list of `2026-11-22-MCP_VIA_LLM_RESPONSES_FOLLOWUPS.md`,
a re-check of rate-limit admission across tool-loop iterations after the
router admission merge, and a bug hunt over `crates/lr-mcp-via-llm` and the
chat / Responses routes.

## Findings (verified in code before changing anything)

1. **Streamed text before the guardrail verdict — confirmed.**
   `orchestrator_stream::streaming_loop` forwards every chunk of the first
   iteration as it arrives and awaits the parallel guardrail gate only after
   the upstream stream ended. A Deny therefore arrives after the client has
   already seen the denied text. Non-streaming awaits the gate before
   anything is returned.
2. **Usage keyed by chunk id — confirmed.** `StreamTracker` keeps the latest
   usage per chunk id and sums across ids. The orchestrator forwards upstream
   chunks with their own ids, so an upstream that reuses one id for every
   call (the route tests' scripted provider does exactly that) has the
   second call's usage overwrite the first's. Prompt-cache accounting and
   gateway costs are keyed the same way and lose calls the same way.
3. **Chat streaming joins iterations — confirmed, and the two paths
   disagree.** Streaming sends every iteration's text as one assistant
   message with nothing between the pieces ("Let me check.The file says");
   non-streaming returns only the last iteration's text. Worse, the client
   sends the joined text back on its next turn, while the session's history
   anchor is the last iteration's text alone: the anchor is not found and the
   hidden tool history is dropped on continuation.
4. **Hidden history not in the session DB — confirmed.** The row stores the
   client-visible history only; the hidden gateway calls/results live in the
   in-memory orchestrator session. Reasoning items from a request's `input`
   (and those a native Responses upstream returned) are not stored either,
   and the streamed row drops the assistant's `reasoning_content`.
5. **Admission across iterations — verified correct, untested.** Each loop
   iteration is its own `Router::complete` / `stream_complete` call: it
   admits (reserves) once and releases when the call's usage is recorded
   (non-streaming: when `complete` returns; streaming: when the stream ends
   or is dropped). The route's `check_rate_limits` is a check, not a
   reservation. No leak and no double count, but no test covered it.
6. **Bug hunt** — see "Bug hunt" below.

## Design

1. While a guardrail gate is pending, the streaming loop holds back every
   chunk it would forward and races the upstream stream against the gate.
   On Allow the held chunks go out at once and the rest streams live (only
   the time to the verdict is added). On Deny nothing has been sent and the
   loop fails with `GuardrailDenied`. The pass-through paths (`tool_choice:
   none`, no gateway tools) get the same gate.
2. The streaming orchestrator gives each LLM call's chunks an id no other
   call of the loop used (it rewrites an id only when an earlier call used
   it), so every per-id map in `StreamTracker` (usage, prompt-cache
   accounting, gateway cost) counts one entry per LLM call.
3. Chat: a choice is one assistant message, so it holds the text of all the
   loop's assistant messages joined by a blank line (`"\n\n"`), streaming and
   non-streaming alike (streaming cannot know which iteration is the last
   without buffering everything). Streaming emits the separator in front of
   the first text after gateway tools ran; non-streaming joins the earlier
   messages' text in front of the final one. The orchestrator history keeps
   the messages apart (the model sees what it wrote), and the session
   remembers the joined text the client received (`reply_text`) as the
   anchor for the next turn. Responses keeps separate `message` items.
4. Responses rows get a nullable `replay_json` column (added by migration):
   - `hidden_history`: the keyed orchestrator session's history for the
     response, without this turn's `instructions` and without the gateway's
     instructions (both are re-added fresh), capped at 4 MiB serialized
     (larger: not stored, the continuation falls back to visible history).
     Messages are stored as the handler sees them, i.e. with secrets as
     vault placeholders, exactly like `messages_json` today.
   - `reasoning_items`: the anchored reasoning items of the request's input
     plus those a native Responses upstream returned, accumulated along the
     chain (bounded to the same cap).
   On a continuation the row's reasoning items are carried with the request
   again, and its hidden history seeds a new keyed orchestrator session when
   no in-memory session carries the key (`TurnOptions::resume_history`).
   Streamed rows keep the assistant's `reasoning_content`.
5. Route test: a tool loop (streaming and not) on a client with a request
   limit; at tool execution time the previous call's reservation is gone,
   and afterwards nothing is in flight and the API key was charged once per
   LLM call.

## Bug hunt

(filled in below as found)

## Checklist

- [x] Plan saved
- [ ] 1 streaming guardrail hold + tests
- [ ] 2 per-call usage ids + tests
- [ ] 3 chat joined assistant text (stream + non-stream, anchor) + tests
- [ ] 4 replay state in the Responses DB + tests
- [ ] 5 admission test
- [ ] 6 bug hunt fixes + tests
- [ ] OpenAPI check
- [ ] Plan Review
- [ ] Test Coverage Review
- [ ] Bug Hunt
- [ ] fmt, clippy, tests
- [ ] Commit and push
