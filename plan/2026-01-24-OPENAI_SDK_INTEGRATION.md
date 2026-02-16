# LLM Tab - OpenAI SDK Integration Plan

## Overview

Replace the current fetch-based approach in the Try It Out LLM tab with the **official OpenAI TypeScript SDK** (`openai` package) for comprehensive LLM API access including chat completions, image generation, embeddings, and vision.

## Why Official OpenAI SDK

| Package | Weekly Downloads | GitHub Stars | Browser Support | Feature Coverage |
|---------|------------------|--------------|-----------------|------------------|
| **openai** (official) | 4.5-9.5M | ~10K | ✅ Yes | Full (chat, images, embeddings, audio, vision) |
| @ai-sdk/openai | 1.8M | - | ✅ Yes | Good (less comprehensive) |
| @langchain/openai | Lower | - | ⚠️ Limited | Chat/embeddings only |

**Chosen: `openai`** - Most comprehensive, best maintained, fully browser-compatible with `dangerouslyAllowBrowser: true`.

---

## Installation

```bash
npm install openai
```

---

## Files to Modify

| File | Changes |
|------|---------|
| `src/views/try-it-out/llm-tab.tsx` | Replace fetch calls with OpenAI SDK, add streaming support |
| `src/lib/openai-client.ts` | **NEW** - Create OpenAI client factory |

---

## Implementation

### 1. Create OpenAI Client Factory

**File:** `src/lib/openai-client.ts`

```typescript
import OpenAI from "openai";

export interface OpenAIClientConfig {
  apiKey: string;
  baseURL: string;
}

export function createOpenAIClient(config: OpenAIClientConfig): OpenAI {
  return new OpenAI({
    apiKey: config.apiKey,
    baseURL: config.baseURL,
    dangerouslyAllowBrowser: true, // Safe for Tauri desktop app
  });
}
```

### 2. Update LLM Tab

**Key changes in `llm-tab.tsx`:**

1. **Import OpenAI SDK** instead of using raw fetch
2. **Add streaming support** with proper React state updates
3. **Create client instance** based on mode (client/strategy/direct)
4. **Use SDK methods** for chat completions

```typescript
import OpenAI from "openai";
import { createOpenAIClient } from "@/lib/openai-client";

// Create client when token/port changes
const openaiClient = useMemo(() => {
  const token = getAuthToken();
  if (!token || !serverPort) return null;

  return createOpenAIClient({
    apiKey: token,
    baseURL: `http://localhost:${serverPort}/v1`,
  });
}, [clientApiKey, strategyToken, internalTestToken, serverPort, mode]);

// Streaming chat completion
const handleSend = async () => {
  if (!openaiClient || !selectedModel) return;

  // Add user message immediately
  setMessages(prev => [...prev, userMessage]);

  // Create assistant message placeholder
  const assistantId = crypto.randomUUID();
  setMessages(prev => [...prev, { id: assistantId, role: "assistant", content: "", timestamp: new Date() }]);

  // Stream response
  const stream = await openaiClient.chat.completions.create({
    model: selectedModel,
    messages: [...messages, userMessage].map(m => ({ role: m.role, content: m.content })),
    stream: true,
  });

  for await (const chunk of stream) {
    const content = chunk.choices[0]?.delta?.content || "";
    setMessages(prev => prev.map(m =>
      m.id === assistantId ? { ...m, content: m.content + content } : m
    ));
  }
};
```

### 3. Future Extensions (SDK enables these)

The OpenAI SDK will make it easy to add:

- **Image Generation** (DALL-E): `openai.images.generate()`
- **Embeddings**: `openai.embeddings.create()`
- **Audio Transcription**: `openai.audio.transcriptions.create()`
- **Vision/Image Input**: Via `image_url` content type in messages

---

## Key Benefits

1. **Streaming by default** - Better UX with token-by-token display
2. **Type safety** - Full TypeScript types for all API parameters/responses
3. **Error handling** - SDK handles retries, rate limits, error parsing
4. **Extensibility** - Easy to add images, embeddings, audio later
5. **Maintainability** - Official SDK stays current with API changes

---

## Browser Security Note

Using `dangerouslyAllowBrowser: true` is safe for Tauri desktop apps because:
- Local desktop application (user-controlled)
- API calls go to localhost (not exposed to internet)
- No risk of key interception via network

---

## Verification

1. [ ] Install `openai` package
2. [ ] Create `src/lib/openai-client.ts`
3. [ ] Update `llm-tab.tsx` to use SDK
4. [ ] Test all three modes (client, strategy, direct)
5. [ ] Verify streaming works with token-by-token display
6. [ ] Test error handling (invalid model, auth failure)

## Sources

- [Official OpenAI SDK - npm](https://www.npmjs.com/package/openai)
- [OpenAI SDK - GitHub](https://github.com/openai/openai-node)
- [OpenAI API Reference](https://platform.openai.com/docs/api-reference)
