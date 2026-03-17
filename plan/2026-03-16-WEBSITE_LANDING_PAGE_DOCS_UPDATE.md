# Plan: Website Landing Page & Docs Update

## Context

Recent implementations (Secret Scanning, Audio endpoints, Moderations, Memory System, enhanced chat params, Elicitation/Sampling) are not reflected on the website landing page or documentation. This plan updates both to match the current state of the codebase.

---

## Part 1: Landing Page (`website/src/pages/Home.tsx`)

### 1A. Hero Subtitle Update (line ~44-46)

Update the hero description to mention audio:

> "Centralized API key storage with per-client access control. Automatic model failover across providers. Audio transcription and speech synthesis. Single Unified MCP Gateway aggregating all MCPs and skills."

### 1B. Free-Tier Mode — Add Popup Infographic

Currently the Free-Tier section (lines ~240-412) has a dark provider-status dashboard on the left. **Add** a small firewall-style popup overlapping or below the dashboard that shows:
- "Free tier exhausted for OpenRouter"
- "Switch to paid?" with Allow / Deny buttons
- Styled like `FirewallApprovalDemo` but simpler — just a static mock

### 1C. Combine Firewall + GuardRails + Secret Scanning → Unified "Safety" Section

**Remove** the two separate sections:
- Feature 6: Firewall (lines ~787-935)
- Feature 6: GuardRails (lines ~937-980)

**Replace with** a single "Safety" section following the MCP Gateway pattern (hero + 4 sub-cards):

#### Hero area:
- Icon: `ShieldCheck` (amber-500)
- Tag: "SAFETY"
- Title: "Runtime Safety Layer"
- Description: "Every request passes through LocalRouter's safety layer — content scanning, secret detection, approval gates, and MCP permission controls. Configurable per client, model, and server."
- Visual: **3 cascading firewall popups** offset to bottom-right of each other:
  - Back popup: GuardRails detection (red tint, "PII Detected in request")
  - Middle popup: Secret scanning (rose tint, "API key detected: sk-...4f")
  - Front popup: Tool approval (amber tint, existing `FirewallApprovalDemo` style for `read_file`)
- "Learn more" → `/docs/firewall`

#### 4 Sub-cards (2x2 grid, same style as MCP Gateway sub-cards):

**Card 1: Content Safety (GuardRails)**
- Icon: `Shield` (red-500)
- Title: "Content Safety Inspection"
- Bullets:
  - Multi-source detection — built-in rules, Presidio, LLM Guard
  - Moderation API — OpenAI-compatible `/v1/moderations` endpoint
  - Parallel scanning — zero-latency on clean requests
- Link: `/docs/guardrails`

**Card 2: Secret Scanning**
- Icon: `ScanSearch` or `KeyRound` (rose-500)
- Title: "Secret Scanning"
- Bullets:
  - Regex + Shannon entropy — multi-stage pipeline catches real secrets
  - Three actions — Ask (popup), Notify (alert), Off — per-client override
  - Time-based bypass — approve once for a configurable period
- Link: `/docs/secret-scanning`

**Card 3: Runtime Approvals**
- Icon: `ShieldCheck` (amber-500)
- Title: "Runtime Approval Firewall"
- Bullets:
  - Request inspection & modification — edit tool args, model params in real time
  - Granular policies — per-client, per-model, per-MCP server, per-skill
  - Allow once / for session / deny
- Link: `/docs/firewall`

**Card 4: Elicitation & Sampling**
- Icon: `MessageSquare` or `MessagesSquare` (indigo-500)
- Title: "Elicitation & Sampling"
- Bullets:
  - Elicitation — MCP servers request structured user input via popup forms (JSON Schema-based)
  - Sampling — MCP servers request LLM completions routed through your providers
  - Gated access — Allow / Ask / Deny policies per server
- Link: `/docs/unified-mcp-gateway` (or whichever doc covers this)

### 1D. New Section: Persistent Memory

Add **after** the Optimizations section (line ~1542) and **before** Privacy (line ~1577).

- Icon: `Brain` (purple-500)
- Tag: "MEMORY"
- Title: "Persistent Conversation Memory"
- Description: "LLMs recall past conversations across sessions. Per-client isolation with automatic transcript capture and semantic search via Zillis memsearch."
- Layout: Text left, visual right
- Bullets:
  - Per-client isolation — each client gets its own memory index and directory
  - Auto-capture — transcripts saved automatically in MCP via LLM and Both modes
  - Semantic recall — MemoryRecall virtual MCP tool with hybrid vector search
  - Session compaction — LLM-powered summarization at session boundaries
- Experimental badge (like the FlaskConical experimental marker on Smart Routing)
- Visual: Dark card showing a memory recall flow:
  - User message: "What did we discuss about rate limiting?"
  - MemoryRecall tool call with search query
  - Retrieved memory snippet from previous session
- Link: `/docs/memory`

### 1E. New Lucide Icons Needed

Add to imports: `ScanSearch` (or `KeyRound`), `MessagesSquare` (or `MessageSquare`), `Brain`

---

## Part 2: Documentation Updates

### 2A. New Doc: Secret Scanning (`website/src/pages/docs/content/19-secret-scanning.md`)

Entries:
- `<!-- @entry secret-scanning-overview -->` — Overview: scans outbound requests for API keys, tokens, passwords before sending to providers
- `<!-- @entry secret-scan-pipeline -->` — Detection pipeline: keyword pre-filter → regex matching → Shannon entropy filtering
- `<!-- @entry secret-scan-categories -->` — Secret categories: Cloud Provider (AWS, GCP, Azure), AI Service (OpenAI, Anthropic), Version Control (GitHub tokens), Database, Financial, OAuth, Generic
- `<!-- @entry secret-scan-actions -->` — Three actions: Ask (popup approval), Notify (alert), Off. Global default with per-client override
- `<!-- @entry secret-scan-approval-flow -->` — Firewall integration: popup shows finding preview, time-based bypass (e.g., "Allow for 1 hour")
- `<!-- @entry secret-scan-allowlist -->` — Tuning: entropy threshold, allowlist regex, custom rules

Reference: `crates/lr-secret-scanner/src/types.rs`, `crates/lr-secret-scanner/src/engine.rs`

### 2B. New Doc: Memory System (`website/src/pages/docs/content/20-memory.md`)

Entries (all marked experimental):
- `<!-- @entry memory-overview -->` — Overview with experimental badge. Persistent conversation memory using Zillis memsearch
- `<!-- @entry memory-architecture -->` — Per-client directory structure: sessions/, archive/, .memsearch/
- `<!-- @entry memory-modes -->` — Supported modes: McpViaLlm (auto-capture), Both (auto-capture via prefix matching). Not supported: LlmOnly, McpOnly
- `<!-- @entry memory-recall-tool -->` — MemoryRecall virtual MCP tool: configurable name, semantic search across past sessions
- `<!-- @entry memory-sessions -->` — Sessions and conversations: 3h inactivity / 8h max duration triggers, conversation grouping within sessions
- `<!-- @entry memory-compaction -->` — LLM-powered summarization: transcript archived, summary replaces in index. Re-compaction support
- `<!-- @entry memory-privacy -->` — Privacy warning: conversations are fully recorded when enabled. UI links to memory folder for review
- `<!-- @entry memory-config -->` — Configuration: embedding model (ONNX or Ollama), session timeouts, compaction LLM provider/model, per-client enablement only

Reference: `plan/2026-03-14-ZILLIS_MEMSEARCH_INTEGRATION.md`, `crates/lr-memory/`

### 2C. Update API Reference (`website/src/pages/docs/content/15-api-openai-gateway.md`)

Add new entries **after** `<!-- @entry openai-embeddings -->`:

- `<!-- @entry openai-audio-transcriptions -->` — `POST /v1/audio/transcriptions`: multipart form-data (file, model, language, prompt, response_format, temperature). Providers: OpenAI, Groq, TogetherAI, DeepInfra. 25MB body limit. Returns `{ text: "..." }`
- `<!-- @entry openai-audio-translations -->` — `POST /v1/audio/translations`: same format, always translates to English. Subset of providers
- `<!-- @entry openai-audio-speech -->` — `POST /v1/audio/speech`: JSON body (model, input, voice, response_format, speed). Returns binary audio. Providers: OpenAI
- `<!-- @entry openai-moderations -->` — `POST /v1/moderations`: JSON body (input, model). Uses configured safety models. Returns OpenAI-compatible moderation response with category scores
- `<!-- @entry openai-image-generations -->` — `POST /v1/images/generations`: JSON body (prompt, model, n, size, quality, style). Provider-dependent

Update existing `<!-- @entry openai-chat-completions -->` to mention new parameters: `n`, `parallel_tool_calls`, `service_tier`, `metadata`, `modalities`, `audio`, `reasoning_effort`, `store`. Update response description to note `system_fingerprint`, `service_tier`, usage in final streaming chunk.

### 2D. Update GuardRails Docs (`website/src/pages/docs/content/10-guardrails.md`)

Add entry after `<!-- @entry parallel-scanning -->`:
- `<!-- @entry moderation-endpoint -->` — Moderation as safety model: OpenAI-compatible `/v1/moderations` endpoint, category mapping to LocalRouter SafetyCategory, pricing fallback for moderation models

### 2E. Update Docs.tsx Sidebar (`website/src/pages/Docs.tsx`)

**Add Secret Scanning section** (after `guardrails`, before `privacy-security` in Security group):
```typescript
{
  id: 'secret-scanning',
  title: 'Secret Scanning',
  icon: <ScanSearch className="h-4 w-4" />,
  subsections: [
    { id: 'secret-scanning-overview', title: 'Overview' },
    { id: 'secret-scan-pipeline', title: 'Detection Pipeline' },
    { id: 'secret-scan-categories', title: 'Secret Categories' },
    { id: 'secret-scan-actions', title: 'Actions (Ask / Notify / Off)' },
    { id: 'secret-scan-approval-flow', title: 'Firewall Approval Flow' },
    { id: 'secret-scan-allowlist', title: 'Allowlist & Tuning' },
  ],
}
```

**Add Memory section** (new sidebar group or under MCP & Extensions):
```typescript
{
  id: 'memory',
  title: 'Memory (Experimental)',
  icon: <Brain className="h-4 w-4" />,
  subsections: [
    { id: 'memory-overview', title: 'Overview' },
    { id: 'memory-architecture', title: 'Architecture' },
    { id: 'memory-modes', title: 'Supported Modes' },
    { id: 'memory-recall-tool', title: 'MemoryRecall Tool' },
    { id: 'memory-sessions', title: 'Sessions & Conversations' },
    { id: 'memory-compaction', title: 'Compaction & Summarization' },
    { id: 'memory-privacy', title: 'Privacy' },
    { id: 'memory-config', title: 'Configuration' },
  ],
}
```

**Update API Reference subsections** (add after `openai-embeddings`):
```typescript
{ id: 'openai-audio-transcriptions', title: 'POST /audio/transcriptions' },
{ id: 'openai-audio-translations', title: 'POST /audio/translations' },
{ id: 'openai-audio-speech', title: 'POST /audio/speech' },
{ id: 'openai-moderations', title: 'POST /moderations' },
{ id: 'openai-image-generations', title: 'POST /images/generations' },
```

**Update GuardRails subsections** (add `moderation-endpoint`):
```typescript
{ id: 'moderation-endpoint', title: 'Moderation Endpoint' },
```

**Update sidebarGroups** to include `secret-scanning` in Security group and `memory` wherever appropriate.

**Add new icon imports**: `ScanSearch`, `Brain`, `MessagesSquare`

---

## Part 3: Implementation Order

1. **Docs.tsx** — Add new sections, sidebar entries, icon imports
2. **19-secret-scanning.md** — New doc file
3. **20-memory.md** — New doc file
4. **15-api-openai-gateway.md** — Add 5 new endpoint entries + update chat completions
5. **10-guardrails.md** — Add moderation entry
6. **Home.tsx** — Hero subtitle update
7. **Home.tsx** — Free-Tier popup addition
8. **Home.tsx** — Merge Firewall + GuardRails into Safety section with 4 sub-cards
9. **Home.tsx** — Add Memory section
10. **Home.tsx** — Icon imports

---

## Part 4: Files to Modify

| File | Action |
|------|--------|
| `website/src/pages/Home.tsx` | Major: merge safety sections, add memory, update hero, add free-tier popup |
| `website/src/pages/Docs.tsx` | Add 2 new sections + update 2 existing sections + icon imports |
| `website/src/pages/docs/content/19-secret-scanning.md` | **New file** |
| `website/src/pages/docs/content/20-memory.md` | **New file** |
| `website/src/pages/docs/content/15-api-openai-gateway.md` | Add 5 endpoint entries + update chat completions |
| `website/src/pages/docs/content/10-guardrails.md` | Add moderation endpoint entry |

---

## Part 5: Verification

1. Run `cd website && npm run dev` to start the dev server
2. Check landing page at `http://localhost:5173/`:
   - Hero subtitle mentions audio
   - Free-Tier section has exhaustion popup
   - Safety section replaces separate Firewall/GuardRails with unified section + 4 sub-cards
   - Memory section appears between Optimizations and Privacy
3. Check docs at `http://localhost:5173/docs/secret-scanning` — all entries render
4. Check docs at `http://localhost:5173/docs/memory` — all entries render
5. Check docs at `http://localhost:5173/docs/api-openai-gateway` — audio, moderations, images entries appear
6. Check docs at `http://localhost:5173/docs/guardrails` — moderation endpoint entry appears
7. Verify sidebar navigation shows new sections in correct groups
8. Check all "Learn more" links from landing page resolve to correct doc sections
