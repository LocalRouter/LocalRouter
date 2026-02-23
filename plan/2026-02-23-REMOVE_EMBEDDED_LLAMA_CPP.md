# Remove Embedded llama.cpp / Local GGUF Execution from Guardrails

**Date**: 2026-02-23
**Status**: In Progress

## Context

The guardrails system currently supports two execution modes: **local GGUF** (embedded llama.cpp with Metal/Vulkan GPU acceleration) and **provider-based** (routing through external LLM providers). The embedded local execution path is being removed entirely — it was never released. Guardrails will exclusively use external LLM providers (Ollama, LM Studio, OpenAI-compatible APIs). This eliminates the large llama-cpp-2 native C++ dependency, Vulkan SDK CI requirements, and simplifies the architecture. Additionally, Ollama's `POST /api/pull` will be exposed so the UI can trigger model downloads through Ollama.

## Phases

1. lr-guardrails crate cleanup (remove llama-cpp-2, downloader, local executor)
2. lr-config types cleanup (remove unreleased local-only fields)
3. Build system & CI (remove Vulkan SDK, metal/vulkan features)
4. Tauri commands (remove 7 local commands, simplify remaining)
5. Ollama model pull support (new feature)
6. Frontend cleanup (types, UI components, demo mocks)
7. Verification
