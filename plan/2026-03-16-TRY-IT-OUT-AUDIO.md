# Plan: Add Audio (TTS + STT) to Try It Out LLM Tab

## Context

Audio endpoints (transcription, translation, speech) were recently implemented on the backend (`crates/lr-server/src/routes/audio.rs`) with provider support for OpenAI, Groq, TogetherAI, and DeepInfra. The Try It Out LLM tab currently has sub-tabs for Chat, Images, and Embeddings but no way to test audio endpoints. This plan adds two new sub-tabs: **Speech** (TTS) and **Transcribe** (STT).

## Approach: Two New Sub-tabs

Two separate panels following the exact same pattern as `ImagesPanel` and `EmbeddingsPanel`:
- Each receives `{ openaiClient, isReady, selectedModel }` props
- Uses the OpenAI JS SDK (already configured) for API calls
- Card-based layout with controls on top, scrollable results history below
- No new backend changes, Tauri commands, or dependencies needed

## Files Changed

| File | Change |
|------|--------|
| `src/views/try-it-out/llm-tab/speech-panel.tsx` | **NEW** — TTS panel |
| `src/views/try-it-out/llm-tab/transcribe-panel.tsx` | **NEW** — STT panel |
| `src/views/try-it-out/llm-tab/index.tsx` | **MODIFY** — add 2 tab triggers + 2 tab contents |
