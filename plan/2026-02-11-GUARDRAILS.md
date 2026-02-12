# GuardRails for LLM Provider Requests

**Date:** 2026-02-11
**Status:** In Progress

## Overview

Optional per-client guardrails that scan both requests and responses for prompt injection, jailbreaks, PII leakage, and code injection. When a rule triggers, a popup (reusing the existing firewall approval flow) shows the detection and lets the user Allow or Deny.

- **Request scanning**: Blocks before sending to provider. Popup lets user Allow/Deny.
- **Response scanning** (configurable): Non-streaming responses block before delivery. Streaming responses scanned incrementally — stream continues flowing, but aborted if rule triggers.

## Source Architecture

Sources are **dynamically downloaded at runtime** (not embedded in binary), following the `lr-marketplace` pattern. Three source types:

- **Regex/Pattern sources** — YARA rules, regex patterns, PII recognizers (fast, <5ms)
- **ML Model sources** — ONNX classification models from HuggingFace (~20-76ms CPU)
- **Built-in** — ~50 hardcoded patterns always available without download

| Source | Status | Type | Default |
|--------|--------|------|---------|
| Built-in rules | Always available | regex | Enabled |
| Microsoft Presidio | Very active | regex | Enabled |
| PayloadsAllTheThings | Very active (75k stars) | regex | Enabled |
| LLM Guard (ProtectAI) | Active (weekly) | regex | Enabled |
| NeMo Guardrails (NVIDIA) | Active (weekly) | regex | Disabled |
| PurpleLlama (Meta) | Active (weekly) | regex | Disabled |
| Meta Prompt Guard 2 | Active | model | Disabled |
| ProtectAI DeBERTa v3 | Active | model | Disabled |

---

## Phase 1: Core Engine + Built-in Rules + Settings UI + Source Framework

### New Crate: `crates/lr-guardrails/`

```
crates/lr-guardrails/
  Cargo.toml
  src/
    lib.rs                # Public API: GuardrailsEngine
    types.rs              # GuardrailCategory, Severity, Match, CheckResult
    engine.rs             # Core engine: loads rules, check_input()/check_output()
    compiled_rules.rs     # CompiledRuleSet wrapping regex::RegexSet + metadata
    text_extractor.rs     # Extract inspectable text from ChatCompletionRequest
    source_manager.rs     # Download, cache, update, hot-reload sources
    sources/
      mod.rs              # GuardrailSource trait + registry
      builtin.rs          # ~50 hardcoded patterns (always available)
      regex_source.rs     # Generic regex pattern file parser
      yara_source.rs      # YARA .yar file parser -> regex extraction
      model_source.rs     # ML model source stub (Phase 2)
```

### Config Changes

- Add `GuardrailsConfig` to `AppConfig` with master toggle, scan toggles, source list, severity threshold, update interval
- Add `GuardrailSourceConfig` for each source (id, label, type, enabled, url, data_paths, branch, predefined)
- Add `guardrails_enabled: Option<bool>` to `Client` (None=inherit global)
- Bump CONFIG_VERSION 6 -> 7 with migration

### Firewall Integration

- Extend `PendingApprovalInfo` with `is_guardrail_request` and `guardrail_details`
- Add `request_guardrail_approval()` to FirewallManager
- Fix 120s timeout bug — all firewall popups wait indefinitely
- Add `GuardrailApprovalTracker` to AppState (time-based bypass like ModelApprovalTracker)

### Route Integration

- `check_request_guardrails()` in chat.rs before provider call (after model firewall check)
- Same check in completions.rs
- Non-streaming response: scan after receive, popup before delivery
- Streaming response: incremental scan every ~500 chars, abort + notification on match

### Frontend

- Guardrail popup: shield icon, severity badges, matched text, Allow/Deny actions
- Settings tab: master toggle, scan toggles, source list with enable/disable/update
- Client config: tri-state guardrails toggle
- Streaming notification: toast on guardrail-response-flagged event

### Tauri Commands

- `get_guardrails_config` / `update_guardrails_config`
- `update_guardrail_source` / `update_all_guardrail_sources`
- `get_guardrail_sources_status`
- `add_guardrail_source` / `remove_guardrail_source`

---

## Phase 2: ML Model Sources (Future)

Feature-gated behind `ml-models` feature flag. ONNX Runtime + HuggingFace tokenizers.

- Meta Prompt Guard 2: 86M BERT, ~350MB, ~20-40ms CPU, 3-class output
- ProtectAI DeBERTa v3: ~400MB, ~40-76ms CPU, binary classification

Both disabled by default due to download size. Combined scoring with regex results.

---

## Critical Files Modified

| File | Change |
|------|--------|
| `crates/lr-guardrails/` (new) | Entire guardrails engine |
| `crates/lr-config/src/types.rs` | GuardrailsConfig, Client.guardrails_enabled |
| `crates/lr-config/src/lib.rs` | v6->v7 migration |
| `crates/lr-mcp/src/gateway/firewall.rs` | Guardrail approval, timeout fix |
| `crates/lr-server/src/state.rs` | GuardrailApprovalTracker, engine in AppState |
| `crates/lr-server/src/routes/chat.rs` | Request/response scanning |
| `crates/lr-server/src/routes/completions.rs` | Request scanning |
| `src-tauri/src/ui/commands_clients.rs` | Guardrail commands |
| `src-tauri/src/main.rs` | Engine initialization |
| `src/views/firewall-approval.tsx` | Guardrail popup |
| `src/components/shared/FirewallApprovalCard.tsx` | Guardrail card type |
| `src/types/tauri-commands.ts` | New types |
| `src/views/settings/guardrails-tab.tsx` (new) | Settings UI |
| `src/views/settings/index.tsx` | Add tab |
| `src/views/clients/tabs/config-tab.tsx` | Per-client toggle |
| `website/src/components/demo/TauriMockSetup.ts` | Demo mocks |
