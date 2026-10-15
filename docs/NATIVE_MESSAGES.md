# Anthropic Messages clients

Use the LocalRouter root URL (for example `http://localhost:3625`) as the
Anthropic SDK base URL. The SDK appends `/v1/messages`. Authenticate with a
LocalRouter client secret using `x-api-key` or `Authorization: Bearer`.
LocalRouter supplies the configured provider's upstream credential.

```python
import os
from anthropic import Anthropic

client = Anthropic(
    base_url="http://localhost:3625",
    api_key=os.environ["LOCALROUTER_CLIENT_SECRET"],
    max_retries=0,
)
message = client.messages.create(
    model="my-anthropic/claude-sonnet-4-5",
    max_tokens=512,
    messages=[{"role": "user", "content": "Hello!"}],
)
print(message.content)
```

Replace the model with an allowed provider instance/model from LocalRouter's
model list. LocalRouter's current permissions, approvals, secret scanning,
safety checks and quotas apply.

Direct Anthropic API-key providers use the native Messages transport. Native
content blocks, cache-control markers, thinking signatures and client-owned tool
history retain their original values and order. Streaming retains native SSE
events, including signature deltas and terminal errors. Zero `max_tokens` is
passed through for prompt-cache warming.

Other chat providers use an explicitly validated translation subset. Unsupported
native fields, protected history and cache semantics return an error. LocalRouter
does not claim native parity across gateways, OAuth providers or beta features.
Provider-side server tools, remote MCP definitions, container references and file
IDs require explicit capability and ownership support and are rejected otherwise.

Native tool/thinking history requires an explicit provider/model, or an established
allowed direct-provider session pin with `localrouter.session.require_continuity`
set to true. Keep subsequent requests on that provider and include its original
content blocks unchanged.
Raw native envelopes bypass LocalRouter's exact response cache.

`POST /v1/messages/count_tokens` supports verified native token-count transports
and requires an explicit provider/model. Its body contains `model`, `messages`
and optional `system`, `tools`, `tool_choice` and `thinking`; generation parameters
are not accepted. The returned `input_tokens` is the provider's token-count
estimate, not generated tokens or an invoice. Direct Anthropic token counting is
free and has separate upstream rate limits. Local client request quotas still
apply.

Try It Out → Messages accepts editable native request JSON, displays full
responses or streamed events and has a separate input-token counting action.
Requests run only when sent explicitly; Stop cancels the connection. A cancelled
generation may already have incurred upstream usage.

References: [Messages API](https://platform.claude.com/docs/en/api/messages/create),
[streaming](https://platform.claude.com/docs/en/build-with-claude/streaming),
[token counting](https://platform.claude.com/docs/en/build-with-claude/token-counting),
[prompt cache pricing](https://platform.claude.com/docs/en/build-with-claude/prompt-caching).
