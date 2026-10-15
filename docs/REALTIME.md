# Native realtime WebSocket sessions

Enable **Allow realtime sessions** in a client's Settings. LocalRouter relays
native JSON text/audio events to configured public OpenAI API-key or LLM Gateway
instances. Other OpenAI-compatible instances and ChatGPT OAuth do not imply
realtime support. Use an explicit qualified model, such as `openai/gpt-realtime`
or `gateway/openai/gpt-realtime`.

Server clients connect to `ws://localhost:3625/v1/realtime?model=PROVIDER/MODEL`
with their normal direct local client secret in `Authorization: Bearer ...`.
`/realtime` is an authenticated alias. There are no credentials in query strings,
no automatic routing, fallback or reconnects. OAuth access tokens and upstream
provider keys cannot authenticate this local endpoint.

Browser clients first POST `/v1/realtime/client_secrets` with the normal local
client Bearer credential and this JSON:

```json
{"session":{"type":"realtime","model":"openai/gpt-realtime"}}
```

The response contains `value`, `expires_at` and the pinned session type/model.
Open a WebSocket using protocols `realtime` and
`openai-insecure-api-key.TICKET_VALUE`. These tickets authenticate only the local
relay. They are single-use, held hashed in memory and expire after 60 seconds;
optional `expires_after` accepts `anchor: "created_at"` and 10–60 seconds.
Unlike some vendors' reusable tickets, one local ticket opens at most one
connection. Opening TTL does not control the lifetime of an established session.
Current client, model, privacy, credential and provider binding checks run again
at upgrade and throughout the relay. Rotation/revocation closes an active
connection. Tickets cannot authenticate other endpoints.

Each connection is bounded to 120 seconds, 1 MiB per frame, 64 MiB total traffic,
20 explicit response creations, and 1,024 output tokens per response. At most two
sessions per client and sixteen globally may run; pending tickets are bounded.
Each connection and explicit response consumes a request-quota unit. Free-only,
cost/token ceilings and strict local/no-retention policies are rejected because
native audio/time billing cannot establish those guarantees before dispatch.
Blocking content-scan policies reject opaque realtime sessions. The connection
approval pipeline does not inspect subsequent audio or conversation frames.

The relay sets manual generation controls and waits for a native acknowledgement
before forwarding client content. SDK messages sent immediately after opening
wait in the socket until this acknowledgement. Session updates must preserve the
model and output cap. VAD requires `create_response: false`; idle-timeout automatic
generation is unsupported. Changes wait for a new native acknowledgement.
Only JSON text frames are supported; audio travels as base64 in
`input_audio_buffer.append`. Native binary frames are rejected.

Supported events include session updates, text conversation items, client
function results, audio append/commit/clear, response creation/cancellation and
conversation item retrieval/deletion/truncation. Functions are executed by the
client, never LocalRouter. Hosted tools, MCP execution, file/image references,
stored prompts, additional transcription models and transcription-only sessions
are outside this initial mapping. WebRTC and SIP are unsupported. Audio settings use built-in native voices and
24 kHz PCM / G.711 formats; custom account-owned voice references are unsupported.

`response.cancel` is forwarded as an actual native event. Disconnect, native
errors, policy changes or local limits close both sides without retry. Closing
the relay cannot undo output already generated or guarantee no native charges.
Native usage events pass through unchanged. Native audio/time usage snapshots
remain in the session monitor and wire events; they are not converted into
text-token dashboard aggregates. Monitor metadata keeps reported
input/output snapshots by response ID; repeating a snapshot does not add another
charge or token count. Missing session usage and fees remain explicitly unknown,
not a text-token conversion or an assumed zero total. Audio/frame payloads are
not recorded by this relay's monitor event.

Try It Out provides connection, text input, native response cancellation and
disconnection. It displays only the last 40 events and short text previews;
audio payloads and tickets are not displayed. Its bundled demo is clearly
labeled and never opens an upstream realtime connection or generates a model
response. Regression validation uses local mock WebSocket servers, not paid calls.
