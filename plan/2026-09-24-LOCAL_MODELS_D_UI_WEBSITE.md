# Phase 2 · Plan D: UI and website for Local Embedded providers

Part of `plan/2026-09-24-LOCAL_MODELS_PHASE2_OVERVIEW.md` (revised 2026-09-24). Consumes the commands and events in plans A-C; can start against demo mocks.

## Step 0: tracking
Todo items per step.

## Step 1: Add Provider dialog
- New first section **"Local Embedded Providers"** above "Local Providers", described as: "LocalRouter runs the engine on your machine and manages models directly in-app: browse and download from Hugging Face, load and unload on demand. Install the engine once with your package manager." Entries: llama.cpp, Laya, Kev.
- Types with `listed == false` are hidden (legacy "llama.cpp server"). Types arrive in a stable order.
- Custom tab picks `openai_compatible` explicitly (fixes the random pick between the two generic types); `systemone_compatible` gets its own card there.
- Adding a Local Embedded provider opens its detail page on the Engine tab.

## Step 2: Engine tab (all Local Embedded providers)
- Status line: "Found llama-server 0.5.0 (build 11146) at /opt/homebrew/bin/llama-server" or "Not found on PATH".
- **Install commands for this OS**, default first, others under "Other ways". Each in a code block with **Copy**. Commands that don't need sudo also get **Install**, which runs exactly that command and streams output into a log panel (Cancel while running). Laya and Kev show the uv step first; Kev shows **Prepare** once uv is found.
- **Refresh** re-detects (re-reads PATH, including the Windows registry PATH).
- Platform notes (Intel Mac: llama.cpp builds from source; Laya and Kev unsupported).
- Running processes for this provider: model, port, memory, uptime, restarts, **Logs**, **Stop**. API keys never shown.

## Step 3: Models tab
- **llama.cpp:** Discover (static Recommended list + Hugging Face search on submit), repo drawer (sanitized model card text, license, gated notice with "Request access on huggingface.co" and sign-in, quant table with size, memory estimate and Fits/Tight/Won't fit, context slider), Downloads (progress, speed, ETA, pause/resume/cancel, retry), Library (state Unloaded/Loading/Loaded/Failed, Load/Unload, Keep loaded, per-model settings, Try it, Verify, Reveal, Delete, Import GGUF…). A Settings sub-tab holds defaults and runtime settings (context, GPU offload, flash attention, KV cache, parallel slots, max loaded models, idle unload, memory guardrail, storage folder).
- **Laya:** checkpoints (english, multilingual, typed-decisions) with size, enabled toggle, state, "Download now"; device and threads.
- **Kev:** checkpoints (0.8b, 4b, 9b) with download size, hardware fit badge, enabled toggle, per-process state.

## Step 4: Hugging Face account card
In each Local Embedded provider's settings and in the llama.cpp repo drawer when needed: "Sign in with Hugging Face" (browser; CIMD client) or "Use an access token" (masked input, link to huggingface.co/settings/tokens, advice for a fine-grained read token with gated-repo access); signed-in view with username, method, expiry, Sign out. One account shared by all Local Embedded providers (Laya/Kev receive it as `HF_TOKEN`).

## Step 5: elsewhere in the app
Try It Out "Loading model…" for cold models; Monitor `local_model_state` events; tray "Local models: N loaded · Unload all"; `ServiceIcon` entries for `llamacpp_embedded`, `laya`, `kev`; guardrails safety-model picker treats `llamacpp_embedded` as pullable.

## Step 6: website
- **CIMD document:** `website/public/oauth/huggingface-client.json`, served at `https://localrouter.ai/oauth/huggingface-client.json` (GitHub Pages serves `.json` as `application/json`; the spec allows any https path). `client_id` equals that URL; `client_name` "LocalRouter"; `client_uri` `https://localrouter.ai`; `redirect_uris` `["http://127.0.0.1/callback", "http://localhost/callback"]`; `token_endpoint_auth_method` `"none"`; `grant_types` authorization_code, refresh_token, device_code; `response_types` `["code"]`. Verify after deployment: `curl -si` returns 200 JSON, and a Hugging Face authorize URL with that client id shows the LocalRouter consent screen.
- Docs: new `21-local-embedded-providers.md` (overview, install commands per OS, models and downloads, sign-in, settings, troubleshooting); updates to `04-providers.md` (Local Embedded category; legacy llama.cpp note), `14-privacy-security.md`, `15-api-openai-gateway.md` (Laya/Kev now Local Embedded).
- Homepage section "Run models directly"; demo mocks for all new commands (never contacting huggingface.co).

## Step 7: verification
`npx tsc --noEmit`; `cd website && npm run build`; manual pass with real engines in `cargo tauri dev --no-watch`.

## Mandatory final steps
Plan review; test-coverage review; bug hunt (listeners cleaned up, output panels bounded, no remote images, Install only for recipe commands); build and commit.
