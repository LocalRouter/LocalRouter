# Chat Component Unification Plan

## Overview

Unify the chat implementation across API Key, Provider, and Model detail pages into a single context-aware `ContextualChat` component. The key requirement is to show users which model is actually being used (especially important for API Key pages where routing may change the model).

## Current State

### Existing Components

1. **ChatInterface** (`src/components/visualization/ChatInterface.tsx`):
   - Generic, reusable chat UI component
   - Props: `onSendMessage`, `placeholder`, `disabled`
   - Handles message history, streaming, markdown rendering
   - Clean and works well - **no changes needed**

2. **Chat Integration in Detail Pages**:
   - **ApiKeyDetailPage**: Uses generic `'gpt-4'`, routing applies, **doesn't show actual model used**
   - **ProviderDetailPage**: Has model dropdown, uses `'{instanceName}/{selectedModel}'` format
   - **ModelDetailPage**: Fixed model, uses `'{providerInstance}/{modelId}'` format

### Key Issue

**API Key page doesn't capture or display which model was actually used after routing.** The backend already returns this in `chunk.model` (from OpenAI SDK response), but the frontend doesn't capture it.

## Backend Status

✅ **Backend already supports this - no changes needed!**

The backend router already returns the actual model used in both streaming and non-streaming responses:
- `src-tauri/src/server/routes/chat.rs:302` - Non-streaming `ChatCompletionResponse.model`
- `src-tauri/src/server/routes/chat.rs:407` - Streaming `ChatCompletionChunk.model`

The OpenAI SDK exposes this via `chunk.model` property in streaming responses.

## Solution Design

### Architecture: Wrapper Component Pattern

Create a `ContextualChat` wrapper component that adds context-specific behavior around the existing `ChatInterface`:

```
ContextualChat (new wrapper)
  ├─ Context Info Display (routing info, warnings)
  ├─ Model Selector (optional, based on context)
  ├─ ChatInterface (existing, unchanged)
  └─ Model Used Display (shows actual model after routing)
```

### Component Props

```typescript
interface ContextualChatProps {
  context: ChatContext;
  disabled?: boolean;
}

type ChatContext =
  | ApiKeyContext
  | ProviderContext
  | ModelContext;

interface ApiKeyContext {
  type: 'api_key';
  apiKeyId: string;
  apiKeyName: string;
  modelSelection: any; // API key's model selection config
}

interface ProviderContext {
  type: 'provider';
  instanceName: string;
  providerType: string;
  models: Array<{ model_id: string; provider_instance: string }>;
}

interface ModelContext {
  type: 'model';
  providerInstance: string;
  modelId: string;
}
```

## Implementation Steps

### Step 1: Create New Chat Components

**File: `src/components/chat/types.ts`**
- Define `ChatContext` types
- Define props interfaces
- Export shared types

**File: `src/components/chat/ModelSelector.tsx`**
- Model dropdown component
- Used in Provider context
- Props: `models`, `selectedModel`, `onModelChange`, `disabled`, `label`

**File: `src/components/chat/ModelUsedDisplay.tsx`**
- Display component showing actual model used
- Shows warning if routing changed the model
- Props: `requestedModel`, `actualModel`, `contextType`

**File: `src/components/chat/ContextualChat.tsx`**
- Main wrapper component
- Manages state: `selectedModel`, `actualModelUsed`, `chatClient`
- Handles context-specific logic:
  - API Key: Generic 'gpt-4' or allow selection, capture actual model
  - Provider: Model dropdown, use `'{instanceName}/{selectedModel}'`
  - Model: Fixed model, use `'{providerInstance}/{modelId}'`
- Renders ModelSelector (conditional), ChatInterface, ModelUsedDisplay

### Step 2: Implement ContextualChat Component

**Key Logic:**

1. **Initialize based on context:**
   ```typescript
   useEffect(() => {
     if (context.type === 'provider') {
       setSelectedModel(context.models[0]?.model_id || null);
     } else if (context.type === 'model') {
       setSelectedModel(context.modelId);
     }
     // For api_key, use generic 'gpt-4'
   }, [context]);
   ```

2. **Load server config and create OpenAI client:**
   - Get server config via `invoke('get_server_config')`
   - For API Key context: use provided API key
   - For Provider/Model: get first enabled API key
   - Create OpenAI client with baseURL pointing to local server

3. **Capture actual model used (CRITICAL):**
   ```typescript
   const handleSendMessage = async (messages, userMessage) => {
     const stream = await chatClient.chat.completions.create({
       model: getModelString(), // Based on context
       messages: [...messages, { role: 'user', content: userMessage }],
       stream: true,
     });

     async function* generateChunks() {
       for await (const chunk of stream) {
         // CAPTURE ACTUAL MODEL FROM FIRST CHUNK
         if (chunk.model && !actualModelUsed) {
           setActualModelUsed(chunk.model);
         }
         const content = chunk.choices[0]?.delta?.content || '';
         if (content) yield content;
       }
     }

     return generateChunks();
   };
   ```

4. **Construct model string based on context:**
   ```typescript
   function getModelString(): string {
     switch (context.type) {
       case 'api_key':
         return 'gpt-4'; // Generic, routing decides
       case 'provider':
         return `${context.instanceName}/${selectedModel}`;
       case 'model':
         return `${context.providerInstance}/${context.modelId}`;
     }
   }
   ```

### Step 3: Migrate Detail Pages

**ApiKeyDetailPage** (lines 33-42, 160-191):
- Remove: `chatClient` state, `handleSendMessage` function
- Replace chat section (lines 267-300) with:
  ```typescript
  <ContextualChat
    context={{
      type: 'api_key',
      apiKeyId: keyId,
      apiKeyName: apiKey.name,
      modelSelection: apiKey.model_selection
    }}
    disabled={!apiKey.enabled}
  />
  ```

**ProviderDetailPage** (lines 42-45, 163-193, 376-391):
- Remove: `selectedModel` state, `chatClient` state, `handleSendMessage` function, model dropdown
- Replace chat section (lines 362-409) with:
  ```typescript
  <ContextualChat
    context={{
      type: 'provider',
      instanceName,
      providerType,
      models
    }}
    disabled={!enabled}
  />
  ```

**ModelDetailPage** (lines 29, 86-116):
- Remove: `chatClient` state, `handleSendMessage` function
- Replace chat section (lines 230-257) with:
  ```typescript
  <ContextualChat
    context={{
      type: 'model',
      providerInstance: model.provider_instance,
      modelId: model.model_id
    }}
  />
  ```

### Step 4: Error Handling & Edge Cases

**Handle these scenarios in ContextualChat:**

1. **Keychain Access Denied:**
   - Catch error when loading API key value
   - Show clear error: "Keychain access required to use chat"
   - Provide retry button

2. **Server Not Running:**
   - Check if server config is available
   - Show: "Server not running. Check Server tab to start it."

3. **No Models Available (Provider context):**
   - Check if `models.length === 0`
   - Show: "No models available for this provider"
   - Provide "Refresh Models" button

4. **No Enabled API Keys (Provider/Model context):**
   - Check if no enabled keys found
   - Show: "No enabled API key available. Create and enable an API key first."

5. **Chat Request Errors:**
   - Catch errors during streaming
   - Display error message from API
   - Allow retry

### Step 5: UI Polish

**ModelUsedDisplay component:**
- Show: "Using model: {actualModelUsed}" after first message
- If routing changed model: Show warning badge
  - E.g., "Requested: gpt-4, Using: openai/gpt-4-turbo"
- Update display on each message (routing can change mid-conversation)

**ContextInfo component (part of ContextualChat):**
- API Key context: Show routing info (from `formatModelSelection()`)
- Provider context: Show "Direct provider access" badge
- Model context: Show "Direct model access" badge

**Model Selector (Provider context):**
- Dropdown above chat interface
- Label: "Select Model"
- Shows all models from provider
- Defaults to first model

## Critical Files

### Files to Create
- `src/components/chat/types.ts` - Type definitions
- `src/components/chat/ModelSelector.tsx` - Model dropdown component
- `src/components/chat/ModelUsedDisplay.tsx` - Display actual model used
- `src/components/chat/ContextualChat.tsx` - Main wrapper component

### Files to Modify
- `src/components/apikeys/ApiKeyDetailPage.tsx` - Replace chat with ContextualChat
- `src/components/providers/ProviderDetailPage.tsx` - Replace chat with ContextualChat
- `src/components/models/ModelDetailPage.tsx` - Replace chat with ContextualChat

### Files to Reference (No Changes)
- `src/components/visualization/ChatInterface.tsx` - Base chat component
- `src-tauri/src/ui/commands.rs` - Tauri commands reference
- `src-tauri/src/server/routes/chat.rs` - Backend chat implementation

## Testing Strategy

### Unit Tests
1. **ContextualChat component:**
   - Renders correctly for each context type
   - Constructs model strings correctly
   - Handles model selection changes
   - Captures actualModelUsed from responses

2. **ModelSelector component:**
   - Renders model options
   - Handles selection changes
   - Disables correctly

### Integration Tests
1. **API Key Context:**
   - Sends messages with 'gpt-4'
   - Captures and displays actual model used
   - Shows routing info

2. **Provider Context:**
   - Model dropdown works
   - Selected model used in requests
   - Direct provider access

3. **Model Context:**
   - No model selection shown
   - Correct model string used
   - Direct model access

### Manual E2E Testing
1. **API Key page:**
   - Start chat, verify message sent
   - Check "Using model: {actual}" appears
   - Verify routing info displayed
   - Test with disabled key

2. **Provider page:**
   - Select different models from dropdown
   - Start chat with each model
   - Verify correct model used
   - Test with disabled provider

3. **Model page:**
   - Start chat
   - Verify no model selection shown
   - Verify correct model used

4. **Error scenarios:**
   - Deny keychain access - verify error shown
   - Stop server - verify error shown
   - Provider with no models - verify message shown

## Migration Path

1. **Create new components** (Step 1-2) - No breaking changes
2. **Migrate ModelDetailPage** (simplest, no model selection)
3. **Migrate ProviderDetailPage** (has model selection)
4. **Migrate ApiKeyDetailPage** (most complex, routing)
5. **Test thoroughly** (Step 5)
6. **Remove old chat code** from detail pages

## Success Criteria

✅ Single ContextualChat component used across all three detail pages
✅ API Key page shows which model was actually used after routing
✅ Provider page has model selection dropdown for that provider only
✅ Model page shows fixed model with no selection
✅ All error scenarios handled gracefully
✅ Routing info displayed appropriately for each context
✅ Code is cleaner with less duplication

## Future Enhancements

- Chat history persistence (localStorage)
- Export conversations
- Model performance metrics (latency, tokens/sec)
- Multi-model comparison view
