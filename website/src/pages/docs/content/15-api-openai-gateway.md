<!-- @entry openai-authentication -->

All OpenAI-compatible endpoints require authentication via a Bearer token in the `Authorization` header. Use your LocalRouter client secret (format: `lr-{random}`) or an OAuth access token obtained from `POST /oauth/token`. Example:

```
Authorization: Bearer lr-your_secret_key_here
```

Requests without a valid token receive a `401 Unauthorized` response.

All endpoints below are served at the root path (e.g., `GET /models`). The `/v1` prefix is also accepted for compatibility with clients that include it (e.g., `GET /v1/models`). Both the OpenAI gateway and MCP gateway share the same root — their endpoints do not conflict.

<!-- @entry openai-models -->

`GET /models` returns a list of all models available to the authenticated client, filtered by the client's strategy permissions. The response follows the OpenAI models list format with `id`, `object`, `created`, and `owned_by` fields.

Model IDs use the format `provider/model_name` (e.g., `openai/gpt-4o`, `anthropic/claude-sonnet-4-20250514`). The special model `localrouter/auto` is included when the client's strategy has auto-routing configured.

<!-- @entry openai-chat-completions -->

`POST /chat/completions` is the primary endpoint for LLM inference. It accepts the standard OpenAI chat completions request format with `model`, `messages`, `temperature`, `max_tokens`, `stream`, `tools`, `n`, `parallel_tool_calls`, `service_tier`, `metadata`, `modalities`, `audio`, `reasoning_effort`, `store`, and other parameters. The `model` field accepts either a specific model ID (`openai/gpt-4o`) or `localrouter/auto` for intelligent routing.

Responses follow the OpenAI format with `choices`, `usage` (token counts), `model` (the actual model used), `system_fingerprint`, and `service_tier`. Streaming is supported via `stream: true`, with `usage` included in the final streaming chunk before `[DONE]`.

<!-- @entry openai-completions -->

`POST /completions` provides the legacy text completions API. It accepts a `prompt` string (instead of `messages`) along with standard parameters like `model`, `max_tokens`, `temperature`, and `stop`. This endpoint is primarily for backward compatibility with older applications.

The response includes `choices` with `text` and `finish_reason` fields. Not all providers support this endpoint — those that don't will return a `400` error.

<!-- @entry openai-embeddings -->

`POST /embeddings` generates vector embeddings for input text. The request includes `model` (must be an embedding model like `openai/text-embedding-3-small`) and `input` (a string or array of strings). The response contains an array of embedding objects, each with a `float[]` vector and token usage data.

Embedding dimensions vary by model. This endpoint is useful for RAG pipelines, semantic search, and similarity comparisons.

<!-- @entry openai-audio-transcriptions -->

`POST /audio/transcriptions` converts audio to text using multipart form-data. The request includes `file` (audio binary, 25MB limit), `model`, and optional `language`, `prompt`, `response_format`, and `temperature` fields. Supported providers include OpenAI, Groq, TogetherAI, and DeepInfra.

The response returns `{ "text": "..." }` with the transcribed content.

<!-- @entry openai-audio-translations -->

`POST /audio/translations` translates audio into English using the same multipart form-data format as transcriptions. It accepts `file`, `model`, `prompt`, `response_format`, and `temperature` fields.

This endpoint always produces English output regardless of the source language. A subset of transcription providers support translations.

<!-- @entry openai-audio-speech -->

`POST /audio/speech` generates audio from text. The JSON request body accepts `model`, `input` (the text to synthesize), `voice`, `response_format`, and `speed` parameters.

The response is a binary audio stream. The `Content-Type` header varies based on the requested output format (e.g., `audio/mpeg`, `audio/opus`).

<!-- @entry systemone -->

`POST /systemone` (also `POST /v1/systemone`) answers typed questions about a situation with calibrated probabilities instead of generated text. It uses the System One wire format of TypeSafe's Jev API. For each question, a System One model returns a probability for every allowed answer: which option applies (`choice`), where on an ordered scale the situation falls (`score`), or how likely a statement is to be true (`noul`). Use it for routing, triage, moderation and gating decisions, where you want a number you can threshold rather than text you have to parse.

**Request**

- `model` (optional): see model resolution below.
- `state`: the situation being judged, as a string or any JSON object or array.
- `questions`: an object keyed by question id. Each question has a `type`, `instructions` and `criteria`:
  - `choice`: `criteria` maps each option key to a description (a string, an object, or `null`). 1-255 options, kept in the order given.
  - `score`: `criteria` is an ordered array of 2-10 level descriptions, lowest first.
  - `noul`: optional `criteria` of the form `{"true": "...", "false": "..."}` describing what yes and no mean.

```json
{
  "model": "Ollaya/laya:en",
  "state": "I was charged twice this month and need a refund before Friday.",
  "questions": {
    "department": {
      "type": "choice",
      "instructions": "Which team should handle this ticket?",
      "criteria": { "billing": "Payments and refunds", "technical": "Bugs and outages", "sales": null }
    },
    "urgency": {
      "type": "score",
      "instructions": "How urgent is this ticket?",
      "criteria": ["Can wait", "Normal", "Needs attention today"]
    },
    "refund_requested": { "type": "noul", "instructions": "Does the customer ask for a refund?" }
  }
}
```

**Response**

The response has one answer per question id, the `model` that answered (as reported by a native provider, or `provider/model` for a translated chat model), and `usage` (`input_tokens` and `output_tokens`, either of which may be `null`). Extra fields returned by the provider are passed through.

```json
{
  "model": "english",
  "answers": {
    "department": { "type": "choice", "choice": "billing", "confidence": 0.82, "probabilities": { "billing": 0.85, "technical": 0.1, "sales": 0.05 } },
    "urgency": { "type": "score", "score": 1.62, "confidence": 0.47, "legend": { "0": "Can wait", "1": "Normal", "2": "Needs attention today" }, "probabilities": { "0": 0.04, "1": 0.3, "2": 0.66 } },
    "refund_requested": { "type": "noul", "noul": 0.95 }
  },
  "usage": { "input_tokens": 96, "output_tokens": 3 }
}
```

`choice` is the most likely option. `score` is the expected level (the sum of each level times its probability). `noul` is the probability of yes. `confidence` measures how concentrated the distribution is.

**Headers**

- `x-localrouter-systemone-backend`: `native` when a System One model answered, or `letter_logprobs` / `json` when LocalRouter translated the questions for a chat model (see below).
- `x-typesafe-request-id`: the upstream request id, when the provider returns one.
- `x-localrouter-generation-id`: the LocalRouter generation id, as on the other endpoints.

**Errors**

Errors use the standard error envelope (`{"error": {"message": "..."}}`):

- `400`: invalid request, such as a missing `state`, no questions, a choice with no options or more than 255, or a score with fewer than 2 or more than 10 levels.
- `401`: missing or invalid key.
- `403`: the client may not use the model, or the firewall, secret scanner or guardrails denied the request.
- `429`: rate limited.
- `502`: provider failure.

An upstream `422` validation error from a System One provider is returned unchanged, including its `detail` body.

**Model resolution**

- `provider/model` (e.g. `Ollaya/laya:en`, `typesafe/jev-latest`): that provider and model.
- A bare model id (e.g. `jev-latest`): the provider serving that model among the client's allowed models.
- `localrouter/auto`: auto-routing over the strategy's prioritized models, skipping models that cannot answer.
- Omitted: if exactly one System One provider is allowed for the client, LocalRouter uses its default model. Otherwise the request is routed as `localrouter/auto`.

**Native and translated backends**

System One providers (TypeSafe Jev, the Local Embedded providers Ollaya, Laya, Kev, Von and Decider, System One compatible servers, the Jev models served by OpenRouter, LLM Gateway, Vercel AI Gateway and Cloudflare Workers AI, and Ollama's decision models such as `nimble` and `tev1`; see System One Providers) answer natively. Any chat model can also answer, because LocalRouter translates the questions into chat completions:

- **Letter mode** (`letter_logprobs`): used where the provider returns token log probabilities (OpenAI, Together AI, llama.cpp, Ollama). The model picks one option letter per question, and the probabilities come from the logprobs of those letters.
- **JSON mode** (`json`): used for all other providers. The model returns a JSON object of probabilities, which LocalRouter normalizes.

Translated answers are billed like the chat calls they make. Set `systemone.emulation: off` in the config to allow only native providers.

**curl**

```bash
curl http://localhost:3625/v1/systemone \
  -H "Authorization: Bearer lr-your_secret_key_here" \
  -H "Content-Type: application/json" \
  -d '{"model": "Ollaya/laya:en", "state": "My invoice is wrong", "questions": {"team": {"type": "choice", "instructions": "Which team should handle this?", "criteria": {"billing": null, "technical": null}}}}'
```

**TypeSafe SDK**

The TypeSafe SDK works unchanged. Set its base URL to LocalRouter and use a LocalRouter client key as the API key:

```bash
pip install typesafe-sdk
export TYPESAFE_BASE_URL=http://localhost:3625
export TYPESAFE_API_KEY=lr-your_secret_key_here
```

Requests the SDK sends to `/v1/systemone` then go through LocalRouter's routing, guardrails, secret scanning, prompt compression and monitoring.

<!-- @entry openai-moderations -->

`POST /moderations` analyzes content for safety concerns using configured GuardRails safety models. The JSON request body accepts `input` (a string or array of strings) and an optional `model` field.

The response follows the OpenAI moderation format with category flags and confidence scores mapped to LocalRouter's safety categories.

<!-- @entry openai-image-generations -->

`POST /images/generations` creates images from text prompts. The JSON request body accepts `prompt`, `model`, `n`, `size`, `quality`, and `style` parameters.

Provider support for image generation varies — not all providers or models support every parameter combination. `size` accepts `auto` or any `WIDTHxHEIGHT` from 64 to 4096 pixels per side; each provider rejects sizes its models cannot produce.

<!-- @entry openai-image-edits -->

`POST /images/edits` changes existing images with a prompt. The request is `multipart/form-data`, as in OpenAI's API: `model` (`provider/model`), `prompt`, one or more images (`image[]`, or `image` for a single one; PNG, JPEG or WebP, up to 20 MB each and 16 images), an optional `mask` (its transparent areas are the ones edited), `n` (1 to 10), `size` (defaults to the first image's size) and `response_format` (`b64_json` or `url`). Several images are passed to the model as references in order, for example to combine a subject with a style.

Image edits need a provider that supports them; today that is the stable-diffusion.cpp Local Embedded provider (for example `stable-diffusion.cpp/qwen-image-2.1` or `stable-diffusion.cpp/flux2-klein-4b`). Other providers answer with a 400 error. Try It Out's Images section has an Edit mode, and any generated image can be sent back to it for further edits.

<!-- @entry openai-health -->

`GET /health` returns the server's health status. This endpoint does not require authentication and is suitable for load balancer health checks and monitoring.

The response includes the server status, uptime, version, and the number of configured providers and clients. A `200` status code indicates the server is healthy and ready to accept requests.

<!-- @entry openai-spec -->

`GET /openapi.json` returns the full OpenAPI 3.0 specification for all LocalRouter endpoints. The spec includes request/response schemas, authentication requirements, and endpoint descriptions.

This spec can be imported into API clients like Postman or used to generate client libraries in any language.

<!-- @entry openai-streaming -->

When `stream: true` is set in a chat completions request, the response uses Server-Sent Events (SSE). Each event contains a `data:` line with a JSON chunk following the OpenAI streaming format: `choices[0].delta` contains incremental content tokens.

The stream begins with a chunk containing the role, continues with content deltas, and ends with a `data: [DONE]` sentinel. Token usage is included in the final chunk before `[DONE]`. The `Content-Type` header is set to `text/event-stream`.

<!-- @entry openai-errors -->

Error responses follow the OpenAI error format with an `error` object containing `message`, `type`, `param`, and `code` fields. Common error codes:

- `401` — Invalid or missing authentication token
- `403` — Client lacks permission for the requested model or action (also used for firewall denials)
- `404` — Model not found or not available to the client
- `429` — Rate limit exceeded (includes `Retry-After` header)
- `500` — Internal server error or upstream provider failure
- `502` — Upstream provider returned an invalid response
- `503` — All providers in the fallback chain are unavailable
