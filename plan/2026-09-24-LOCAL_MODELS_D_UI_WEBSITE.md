# Phase 2 · Plan D: UI and website for Local Models

Part of `plan/2026-09-24-LOCAL_MODELS_PHASE2_OVERVIEW.md`. Consumes the Tauri commands, types and events defined in plan B Step 7 (can start against mocks in `website/src/components/demo/TauriMockSetup.ts` before the backend lands).

## Goal

One place to find, download, run and manage local models, with honest "will this fit" guidance, a clear engine story, optional Hugging Face sign-in, and no network traffic until the user acts.

## Step 0: tracking
- Todo items per step; save this plan with `./copy-plan.sh`.

## Step 1: navigation and entry points
- New view `local-models` (`src/views/local-models/index.tsx`), registered in `src/App.tsx` and as a static child of the "LLMs" collapsible in `src/components/layout/sidebar.tsx` (same pattern as `mcpStaticChildren`), plus a command-palette entry.
- The built-in provider's detail page (providers panel, `provider_type === "localrouter_local"`) shows a summary card (engine, loaded models, disk use) with "Open Local Models", and hides the generic settings form (it has no setup parameters).
- The "Add provider" dialog lists "Local models (built-in)" under Local; choosing it enables the feature and opens the view.
- First-run empty state: explains what local models are, shows detected hardware ("Apple M3 Pro, 36 GB unified memory, Metal"), and two actions: "Browse recommended models" and "Install engine" (with size). Nothing is downloaded until clicked.

## Step 2: tabs of the Local Models view
**Discover**
- Two sections: **Recommended** (static curated list shipped in the app: a few chat models per memory tier, an embedding model, a vision model, Tev1 GGUF for System One letter mode, Laya, Kev) and **Search Hugging Face** (runs only when the user submits a query).
- Search controls: text; type (Chat, Embedding, Vision, Reranker, System One); size range (parameter count); sort (downloads, likes, trending, recently updated); "GGUF only" (default on).
- Result rows: repo, author, parameter size, downloads, likes, updated, license, gated badge, pipeline tag. Rate-limit errors show "Hugging Face is rate limiting searches; sign in to raise the limit or try again in a few minutes".
- **Repo drawer** (on click; fetches details then): sanitized model card as text (markdown rendered with images and remote embeds removed, links open the system browser), license, gated notice with "Request access on huggingface.co" (opens browser) and "Sign in" when relevant, and a **file picker table**: quant, size, memory estimate, fit badge (Fits / Tight / Won't fit, colour-coded, tooltip with weights + KV + overhead and the context assumed), recommended quant highlighted (largest that Fits; Q4_K_M if several), vision projector auto-included, split files grouped. A context-length slider recomputes the fit live (runs `hf_inspect_file` once per file for the header, then local math).
- "Download" adds a job and switches focus to Downloads; if the engine is not installed, offers to install it in the same step (two separate size figures shown).

**Library**
- Table of installed models: name (alias editable), kind icons (chat, tools, vision, embedding, System One), quant, size on disk, context, state (Unloaded / Loading… / Loaded with memory / Failed with reason), last used.
- Row actions: Load / Unload, "Keep loaded" toggle, **Settings** (per-model overrides drawer: context, GPU offload, flash attention, KV cache type, parallel slots, batch sizes, mmap; each shows "Default (x)" until changed, with live fit re-estimate), "Try it" (opens Try It Out with `localrouter_local/<id>` selected), Verify files, Reveal in folder, Delete (Radix `AlertDialog`, shows freed space).
- Footer: total disk use, storage folder, "Import GGUF file…" (native file picker via Tauri dialog plugin; validates the header before adding), "Unload all".

**Downloads**
- Active and paused jobs with per-file and total progress bars, speed, ETA, bytes; Pause / Resume / Cancel; errors with Retry (checksum mismatch, disk full, gated access, network). Jobs left unfinished from a previous run appear paused, never auto-resumed.
- Reuse `src/components/shared/ModelDownloadCard.tsx` styling and extend `useModelDownload` with an `eventFilter` on `job_id` (the hook already supports filters for provider pulls).

**Engines**
- Detected hardware card (GPUs with VRAM from the engine's device list once installed; RAM; recommended backend and why).
- Engine packs for this platform: llama.cpp (Metal / Vulkan / CUDA 12 / CUDA 13 / CPU as applicable), Laya, Kev; each with version, size, installed state, "Install", "Remove" (disabled while running), and an "Update available" note after app updates bring a newer pinned build.
- Running processes: model, engine, port (read-only), memory, uptime, restart count, "View logs" (ring buffer, copy button; the API key is never shown), "Stop".

**Settings**
- Storage folder (change with "Move existing models" or "Start empty"; disabled during downloads).
- Defaults for new models: context (Auto / Trained / fixed), GPU offload, flash attention, KV cache type, parallel slots.
- Runtime: max loaded models, idle unload (Never / 5 / 15 / 30 / 60 min), preload on startup (multi-select from Library), engine backend preference, start timeout.
- Memory guardrail: Off / Relaxed / Balanced / Strict with one-line explanations.
- Downloads: concurrent downloads.
- Hugging Face: endpoint (advanced, for mirrors), account card (below).

## Step 3: Hugging Face account card
- Signed out: "Sign in with Hugging Face" (browser OAuth; shows "Waiting for browser…" with Cancel, polling like `OAuthSettingsControls.tsx`) and "Use an access token instead" (masked input with eye toggle, link to `https://huggingface.co/settings/tokens`, advice to create a fine-grained read token with gated-repo read access).
- Signed in: username, method, scopes, expiry (OAuth), "Sign out". Explains that sign-in is only needed for gated or private models and raises search rate limits.
- Reused in the Discover drawer when a gated repo needs it.

## Step 4: touches elsewhere in the app
- **Try It Out:** local models appear in the model picker under their provider; when a cold model is selected, show "Loading model into memory…" driven by `local-models-model-state`; errors like "insufficient memory" show the Library "Settings" shortcut.
- **Monitor:** `llm_call` events already cover requests. Add `local_model_state` events (load, unload, crash, eviction) to the event type union and a compact detail view (model, engine, duration, memory, reason).
- **Tray:** "Local models: N loaded" line with "Unload all" (only when the feature is enabled), per `src-tauri/src/ui/tray_menu.rs` patterns.
- **Guardrails:** `SafetyModelPicker` adds `localrouter_local` to `PULLABLE_PROVIDER_TYPES` and uses the provider's pull (M5).
- **Strategy model selector:** local models carry their capabilities; no change beyond existing chips.
- **Icons:** `ServiceIcon` entry for `localrouter_local` (bundled asset or emoji; no external assets).

## Step 5: types, mocks, accessibility
- Types in `src/types/tauri-commands.ts` for every command in plan B Step 7 (response types at the top, `…Params` at the bottom, camelCase params).
- Demo mocks in `website/src/components/demo/TauriMockSetup.ts` and `mockData.ts`: hardware (Apple Silicon 36 GB), a few curated results, a repo with a quant table and fit badges, one running download (simulated progress timer), a library with one loaded chat model, one embedding model and Laya, engine packs, a signed-out HF account. The demo never contacts huggingface.co.
- Keyboard and screen-reader labels on progress bars, fit badges (text, not colour only) and dialogs.

## Step 6: website
- Docs (`website/src/pages/docs/content/`): new file `21-local-models.md` with entries `local-models-overview`, `local-models-download` (search, quants, fit, gated models, sign-in), `local-models-engines` (why separate engines, backends, updates, privacy), `local-models-settings` (every setting and default), `local-models-systemone` (Laya, Kev, platform matrix), `local-models-troubleshooting` (engine won't start, GPU not used, out of memory, antivirus on Windows, checksum failures). Sidebar entries in `website/src/pages/Docs.tsx`.
- Update `04-providers.md` (built-in provider), `14-privacy-security.md` (what network calls local models make and when; tokens in keychain; engines offline and loopback-only), `15-api-openai-gateway.md` (model naming `localrouter_local/<id>`).
- Homepage (`website/src/pages/Home.tsx`): a "Run models locally" section (download from Hugging Face, fit guidance, GPU acceleration, same routing and guardrails), inline SVG, no external assets; update the provider count wording if needed.
- Demo: the Local Models view works fully against mocks.

## Step 7: verification
- `npx tsc --noEmit` (app), `cd website && npm run build`; manual pass in `cargo tauri dev --no-watch` of every tab against a real engine and a small model (with Try It Out chat, embeddings and a System One request), and the demo at `/demo`.

## Mandatory final steps
1. Plan review; 2. test-coverage review (component tests where the repo has them; otherwise typed mocks and manual script); 3. bug hunt (listeners cleaned up on unmount, progress events filtered per job, disabled states during downloads/engine runs, no external image loads from model cards); 4. tsc/build and commit.
