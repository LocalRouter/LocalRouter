# Provider Endpoint Coverage Matrix

Reference for which providers support which OpenAI-compatible endpoints natively.
Source: Research conducted March 2026 from vendor documentation.

## Summary Matrix

| Provider | STT | TTS | Responses | Moderations | Files | Batches | Realtime WS |
|----------|-----|-----|-----------|-------------|-------|---------|-------------|
| OpenAI | Y | Y | Y | Y | Y | Y | Y |
| Groq | Y | Y | Y | N | Y | Y | N |
| TogetherAI | Y | Y | N | N | Y | Y | Y |
| DeepInfra | Y | N | N | N | N | N | N |
| xAI | N | N* | Y | N | N | N | N |
| LocalAI | Y* | Y* | N | N | N | N | Y |
| Anthropic | N | N | N | N | N | N | N |
| Cerebras | N | N | N | N | N | N | N |
| Cohere | N | N | N | N | N | N | N |
| Gemini | N** | N** | N | N | N | N | N** |
| GPT4All | N | N | N | N | N | N | N |
| Jan | N | N | N | N | N | N | N |
| LlamaCpp | N | N | N | N | N | N | N |
| LM Studio | N | N | N | N | N | N | N |
| Mistral | N | N | N | N | N | N | N |
| Ollama | N | N | N | N | N | N | N |
| OpenRouter | N | N | N | N | N | N | N |
| Perplexity | N | N | N | N | N | N | N |
| OpenAI-compat | * | * | * | * | * | * | * |

`*` = Depends on upstream target
`N*` = Has capability but NOT via OpenAI-compatible endpoint (uses own API)
`N**` = Gemini has STT/TTS/Live via its own native API, not OpenAI-compatible
`Y*` = LocalAI STT/TTS primarily via Realtime pipeline

## Detailed Notes Per Endpoint

### Audio STT (`/v1/audio/transcriptions`, `/v1/audio/translations`)
- **OpenAI**: Official Whisper + GPT-4o transcription/translation
- **Groq**: Whisper Large V3 at `https://api.groq.com/openai/v1/audio/transcriptions`
- **DeepInfra**: OpenAI-compatible at `https://api.deepinfra.com/v1/openai/audio/transcriptions`
- **TogetherAI**: `client.audio.transcriptions.create(...)` with `openai/whisper-large-v3`
- **LocalAI**: Via Realtime pipeline with Whisper STT component

### Audio TTS (`/v1/audio/speech`)
- **OpenAI**: Official at `/v1/audio/speech` (tts-1, tts-1-hd, gpt-4o-mini-tts)
- **Groq**: At `https://api.groq.com/openai/v1/audio/speech`
- **TogetherAI**: At `https://api.together.xyz/v1/audio/speech`
- **LocalAI**: Via Realtime pipeline TTS component
- **xAI**: Has TTS but at `/v1/tts` (NOT OpenAI-compatible path)
- **Gemini**: TTS via own `:generateContent` endpoint, not `/v1/audio/speech`

### Responses API (`/v1/responses`)
- **OpenAI**: Official endpoint
- **Groq**: Full support at `/openai/v1/responses`
- **xAI**: Compatible via OpenAI SDK with `base_url="https://api.x.ai/v1"`

### Moderations (`/v1/moderations`)
- **OpenAI**: Only provider with canonical `/v1/moderations`
- Others use safety models via regular completion endpoints (Groq: Llama Guard; TogetherAI: safety models)

### Files (`/v1/files/*`)
- **OpenAI**: Full Files API
- **TogetherAI**: Files API for fine-tuning and batches
- **Groq**: Files API for batch processing

### Batches (`/v1/batches/*`)
- **OpenAI**: Full Batch API
- **TogetherAI**: Batch API with Files integration
- **Groq**: Batch API at `/openai/v1/batches`

### Realtime (`wss /v1/realtime`)
- **OpenAI**: WebSocket at `wss://api.openai.com/v1/realtime?model=...`
- **TogetherAI**: WebSocket at `wss://api.together.ai/v1/realtime?model=...`
- **LocalAI**: WebSocket at `ws://localhost:8080/v1/realtime?model=...`
- **Gemini Live**: Own WebSocket API (NOT OpenAI-compatible protocol)
- **xAI**: Own voice/agent API (NOT OpenAI `/v1/realtime` schema)
