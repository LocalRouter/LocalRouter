# Secret Scanning for LLM Calls

**Date**: 2026-03-14
**Status**: Planning
**Category**: Optimize Feature

## Goal

Add a new "Secret Scanning" optimize feature that scans outbound LLM request messages for leaked secrets (API keys, tokens, passwords, connection strings, etc.) before they reach providers. This prevents accidental secret exfiltration through LLM conversations — a real risk when developers paste code, configs, or logs into AI tools.

## Architecture Overview

```
Client Request
    ↓
┌─────────────────────────────────────────────────┐
│              Secret Scanner                      │
│                                                  │
│  1. Regex Engine (fast, first pass)              │
│     ├── Built-in patterns (betterleaks-derived)  │
│     ├── Additional curated sources (toggleable)  │
│     └── User custom regex rules                  │
│                                                  │
│  2. Entropy Filter (Shannon entropy on matches)  │
│     └── Configurable threshold per rule          │
│                                                  │
│  3. Optional: ML Model (DistilBERT, second pass) │
│     └── Runs only on regex matches for precision │
│     └── Provider-routed (like guardrails)        │
│                                                  │
│  Actions: block / mask / notify / allow          │
└─────────────────────────────────────────────────┘
    ↓
Guardrails → Compression → Provider
```

## Detailed Design

### 1. New Crate: `lr-secret-scanner`

```
crates/lr-secret-scanner/
├── Cargo.toml
└── src/
    ├── lib.rs              # Public API
    ├── engine.rs           # SecretScanEngine - orchestrates scan pipeline
    ├── regex_engine.rs     # Compiled regex set, pattern matching
    ├── entropy.rs          # Shannon entropy calculator + token efficiency
    ├── patterns/
    │   ├── mod.rs          # Pattern source registry
    │   ├── builtin.rs      # Embedded betterleaks-derived patterns (TOML)
    │   └── custom.rs       # User-defined custom rules
    ├── ml_verifier.rs      # Optional ML model verification (provider-routed)
    ├── actions.rs          # Action resolution (block/mask/notify/allow)
    └── types.rs            # SecretFinding, ScanResult, etc.
```

#### Core Types

```rust
/// A single secret detection rule
pub struct SecretRule {
    pub id: String,
    pub description: String,
    pub regex: String,              // Compiled at load time
    pub secret_group: usize,        // Capture group containing the secret (default: 0)
    pub entropy: Option<f32>,       // Minimum Shannon entropy threshold
    pub keywords: Vec<String>,      // Fast pre-filter keywords
    pub category: SecretCategory,   // For grouping in UI
    pub enabled: bool,
}

/// Categories of secrets (maps to betterleaks rule IDs)
pub enum SecretCategory {
    CloudProvider,      // AWS, GCP, Azure
    AIService,          // OpenAI, Anthropic, Groq, etc.
    VersionControl,     // GitHub, GitLab tokens
    Database,           // Connection strings, passwords
    Financial,          // Stripe, Coinbase
    OAuth,              // OAuth tokens, refresh tokens
    Generic,            // Generic API keys, passwords
    Custom,             // User-defined
}

/// Result of scanning a single message
pub struct ScanResult {
    pub findings: Vec<SecretFinding>,
    pub scan_duration_ms: u64,
    pub rules_evaluated: usize,
}

pub struct SecretFinding {
    pub rule_id: String,
    pub category: SecretCategory,
    pub message_index: usize,       // Which message in the conversation
    pub char_range: (usize, usize), // Location in message content
    pub matched_text: String,       // The matched secret (for masking)
    pub entropy: f32,               // Calculated entropy
    pub confidence: f32,            // 0.0-1.0, boosted by ML if available
    pub ml_verified: Option<bool>,  // None if ML not run, Some(true/false)
}

/// What to do when a secret is found
pub enum SecretAction {
    /// Block the request entirely (return error to client)
    Block,
    /// Replace the secret with [REDACTED] or a placeholder
    Mask,
    /// Allow but emit a notification/event to the UI
    Notify,
    /// Allow without any action
    Allow,
}
```

#### Regex Engine

- **Compile once**: All enabled patterns compiled into a `RegexSet` at startup/config-reload for O(n) scanning where n = input length
- **Keyword pre-filter**: Before regex, check if any rule keywords appear in the text (fast string search via `aho-corasick`). Skip rules whose keywords don't match.
- **Pattern sources**:
  1. **Built-in (betterleaks-derived)**: ~200 rules embedded as a TOML constant, covering AWS, GCP, Azure, GitHub, GitLab, Anthropic, OpenAI, Stripe, database URIs, generic API keys, etc.
  2. **Additional sources** (future): Could add gitleaks, trufflehog pattern sets as toggleable sources
  3. **Custom rules**: Users define via config YAML with same schema

#### Entropy Calculator

```rust
/// Shannon entropy of a string (bits per character)
pub fn shannon_entropy(s: &str) -> f32 {
    let mut freq = [0u32; 256];
    let len = s.len() as f32;
    for &b in s.as_bytes() {
        freq[b as usize] += 1;
    }
    freq.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f32 / len;
            -p * p.log2()
        })
        .sum()
}
```

- Each rule can specify a minimum entropy threshold (typically 3.0-4.5)
- Matches below the threshold are discarded (likely false positives — placeholder values, example tokens)
- Default entropy threshold: 3.5 (configurable globally)

#### ML Verifier (Optional Add-on)

Follows the same pattern as guardrails — routes inference through an external LLM provider:

- **Model**: DistilBERT-based secret classifier (e.g., `betterleaks/distilbert-secret-masker` or similar HuggingFace model)
- **When**: Only runs on regex matches (not on all text) — keeps cost/latency low
- **How**: Provider-routed via the same `ModelExecutor` pattern from `lr-guardrails`
- **Result**: Binary classification (secret/not-secret) with confidence score
- **Configuration**: Optional — regex+entropy alone provides good baseline

The ML verifier is configured similarly to guardrails safety models:
```yaml
secret_scanning:
  ml_verifier:
    enabled: false
    provider_id: "ollama"
    model_name: "betterleaks/distilbert-secret-masker"
```

### 2. Configuration

#### Global Config (`AppConfig`)

```rust
/// Secret scanning configuration
pub struct SecretScanningConfig {
    /// Master enable/disable
    pub enabled: bool,                          // default: false

    /// Default action when a secret is detected
    pub default_action: SecretAction,           // default: Notify

    /// Minimum Shannon entropy for a match to be considered valid
    pub default_entropy_threshold: f32,         // default: 3.5

    /// Built-in pattern sources and their enabled state
    pub pattern_sources: Vec<PatternSourceConfig>,

    /// Per-category action overrides
    pub category_actions: Vec<SecretCategoryAction>,

    /// Custom user-defined rules
    pub custom_rules: Vec<CustomSecretRule>,

    /// ML verifier configuration (optional precision boost)
    pub ml_verifier: Option<SecretMlVerifierConfig>,

    /// Whether to scan system messages (may contain intentional secrets)
    pub scan_system_messages: bool,             // default: false

    /// Whether to scan tool/function call results
    pub scan_tool_results: bool,                // default: true

    /// Allowlist patterns (regexes that exclude matches)
    pub allowlist: Vec<String>,
}

pub struct PatternSourceConfig {
    pub id: String,         // e.g., "betterleaks"
    pub name: String,       // Display name
    pub enabled: bool,
    pub version: String,    // Pattern set version
}

pub struct SecretCategoryAction {
    pub category: SecretCategory,
    pub action: SecretAction,
}

pub struct CustomSecretRule {
    pub id: String,
    pub description: String,
    pub regex: String,
    pub entropy: Option<f32>,
    pub keywords: Vec<String>,
    pub action: Option<SecretAction>,  // None = use default_action
    pub enabled: bool,
}

pub struct SecretMlVerifierConfig {
    pub enabled: bool,
    pub provider_id: String,
    pub model_name: String,
    pub confidence_threshold: f32,     // default: 0.7
}
```

#### Per-Client Config

```rust
pub struct ClientSecretScanningConfig {
    /// None = inherit global, Some(true) = force on, Some(false) = force off
    pub enabled: Option<bool>,

    /// Override default action for this client
    pub default_action: Option<SecretAction>,

    /// Per-category action overrides for this client
    pub category_actions: Option<Vec<SecretCategoryAction>>,
}
```

#### YAML Config Example

```yaml
secret_scanning:
  enabled: true
  default_action: notify
  default_entropy_threshold: 3.5
  scan_system_messages: false
  scan_tool_results: true

  pattern_sources:
    - id: betterleaks
      name: "Betterleaks Patterns"
      enabled: true
      version: "2026.03"

  category_actions:
    - category: cloud_provider
      action: block
    - category: ai_service
      action: mask
    - category: database
      action: block
    - category: generic
      action: notify

  custom_rules:
    - id: internal-api-key
      description: "Internal API key format"
      regex: 'INTERNAL_[A-Za-z0-9]{32}'
      entropy: 3.0
      keywords: ["INTERNAL_"]
      action: block
      enabled: true

  ml_verifier:
    enabled: false
    provider_id: ollama
    model_name: "betterleaks/distilbert-secret-masker"
    confidence_threshold: 0.7

  allowlist:
    - "sk-ant-api03-example.*"
    - "AKIA0000000000EXAMPLE"

# Per-client override
clients:
  - name: "cursor"
    secret_scanning:
      enabled: true
      default_action: mask
      category_actions:
        - category: ai_service
          action: allow   # Cursor legitimately sends AI keys
```

### 3. Integration Points

#### Request Pipeline (`chat.rs`)

Secret scanning runs **in parallel** with guardrails and compression (all are independent pre-processing steps):

```rust
// In handle_chat_completions (non-streaming path)
let (guardrails_result, compression_result, secret_scan_result) = tokio::join!(
    run_guardrails_scan(&state, &config, &request, client_context.as_ref()),
    run_prompt_compression(&state, &config, &request, client_context.as_ref()),
    run_secret_scan(&state, &config, &request, client_context.as_ref()),
);

// Process secret scan result
match secret_scan_result? {
    Some(result) if result.has_blocking_findings() => {
        return Err(/* 400 with details of blocked secrets */);
    }
    Some(result) if result.has_masking_findings() => {
        request = result.apply_masking(request);
        // Also emit notification event
    }
    Some(result) if result.has_notify_findings() => {
        // Emit event to UI only
        emit_secret_scan_event(&state, &result);
    }
    _ => {} // No findings or scanning disabled
}
```

#### AppState

```rust
pub struct AppState {
    // ... existing fields ...
    pub secret_scanner: Arc<RwLock<Option<Arc<lr_secret_scanner::SecretScanEngine>>>>,
}
```

#### Events (UI Notifications)

When `action: notify`, emit a Tauri event so the UI can show a toast/alert:

```rust
pub struct SecretScanEvent {
    pub client_id: String,
    pub findings: Vec<SecretFindingSummary>,
    pub action_taken: String,  // "blocked", "masked", "notified"
    pub timestamp: String,
}
```

### 4. UI Changes

#### Optimize Overview Page

Add a 7th card to the optimize overview grid:

```
┌─────────────────────────────────┐
│ 🔒 Secret Scanning             │
│                                 │
│ [Toggle: On/Off]                │
│                                 │
│ Regex-based secret detection    │
│ with entropy filtering          │
│                                 │
│ [Configure →]                   │
└─────────────────────────────────┘
```

- Icon: `KeyRound` or `ShieldAlert` (Lucide) in orange/rose color
- Toggle for quick enable/disable
- "Configure" button navigates to dedicated settings view

#### Secret Scanning Settings View (`src/views/secret-scanning/`)

Tabbed interface:

**Tab 1: General**
- Enable/disable toggle
- Default action selector (Block / Mask / Notify / Allow)
- Entropy threshold slider (2.0 - 5.0)
- Checkboxes: Scan system messages, Scan tool results

**Tab 2: Pattern Sources**
- List of available pattern sources with toggles
  - Betterleaks Patterns (200+ rules) — [Enabled]
  - (Future: Gitleaks, TruffleHog patterns)
- Expandable sections showing categories within each source
- Per-category action override dropdowns

**Tab 3: Custom Rules**
- Table of user-defined rules
- Add/Edit/Delete with form:
  - ID, Description, Regex (with live tester), Keywords, Entropy threshold, Action
- Import/Export as TOML

**Tab 4: ML Verifier (Advanced)**
- Enable/disable ML precision boost
- Provider + Model selector (same pattern as guardrails model config)
- Confidence threshold slider
- Test button to verify model availability

**Tab 5: Allowlist**
- Regex patterns that exclude matches
- Common presets (example tokens, test fixtures)

#### Per-Client Override

In the client edit view, add a "Secret Scanning" section:

```
Secret Scanning: [Default ▾] [On] [Off]
                 └─ When "On" or "Default":
                    Default Action: [Inherit Global ▾] [Block] [Mask] [Notify]
                    Category Overrides: [Configure →]
```

#### OptimizeDiagram Update

Add "Secret Scan" node in the LLM pipeline, positioned before GuardRails:

```
Client → [Secret Scan] → [GuardRails] → [Compress] → [Strong/Weak] → Provider
```

### 5. Tauri Commands

```rust
// Get/update global config
#[tauri::command]
pub async fn get_secret_scanning_config() -> Result<SecretScanningConfig, String>

#[tauri::command]
pub async fn update_secret_scanning_config(config: SecretScanningConfig) -> Result<(), String>

// Pattern source management
#[tauri::command]
pub async fn get_secret_scanning_sources() -> Result<Vec<PatternSourceInfo>, String>

// Custom rule management
#[tauri::command]
pub async fn test_secret_rule(regex: String, test_input: String) -> Result<Vec<TestMatch>, String>

// Manual scan for testing
#[tauri::command]
pub async fn test_secret_scan(input: String) -> Result<ScanResult, String>

// Get scan statistics/history
#[tauri::command]
pub async fn get_secret_scan_stats() -> Result<ScanStats, String>
```

### 6. Implementation Phases

#### Phase 1: Core Engine + Regex Scanning
- Create `lr-secret-scanner` crate
- Implement regex engine with betterleaks-derived patterns
- Implement Shannon entropy filtering
- Implement keyword pre-filtering with aho-corasick
- Add `SecretScanningConfig` and `ClientSecretScanningConfig` to config types
- Config migration to v22
- Integrate into chat request pipeline (parallel with guardrails)
- Action handling: block, mask, notify, allow
- Unit tests for pattern matching and entropy

#### Phase 2: UI + Configuration
- Optimize overview card
- Secret scanning settings view (General, Sources, Custom Rules, Allowlist tabs)
- Per-client override in client edit view
- Tauri commands for config CRUD
- TypeScript types for all config/response types
- Demo mock handlers
- OptimizeDiagram update

#### Phase 3: ML Verifier Add-on
- ML verifier integration using provider-routed inference
- Settings UI tab for ML configuration
- Confidence threshold tuning
- Test/validate model availability command

#### Phase 4: Polish + Advanced Features
- Scan statistics and history tracking
- Allowlist presets (test tokens, example values)
- Rule import/export (TOML format)
- Pattern source auto-update mechanism (future)
- Response scanning (scan provider responses for leaked secrets echoed back)

### 7. Files to Create/Modify

**New Files:**
- `crates/lr-secret-scanner/Cargo.toml`
- `crates/lr-secret-scanner/src/lib.rs`
- `crates/lr-secret-scanner/src/engine.rs`
- `crates/lr-secret-scanner/src/regex_engine.rs`
- `crates/lr-secret-scanner/src/entropy.rs`
- `crates/lr-secret-scanner/src/patterns/mod.rs`
- `crates/lr-secret-scanner/src/patterns/builtin.rs`
- `crates/lr-secret-scanner/src/patterns/custom.rs`
- `crates/lr-secret-scanner/src/ml_verifier.rs`
- `crates/lr-secret-scanner/src/actions.rs`
- `crates/lr-secret-scanner/src/types.rs`
- `src/views/secret-scanning/index.tsx`
- `src/views/secret-scanning/GeneralTab.tsx`
- `src/views/secret-scanning/SourcesTab.tsx`
- `src/views/secret-scanning/CustomRulesTab.tsx`
- `src/views/secret-scanning/AllowlistTab.tsx`
- `src/views/secret-scanning/MlVerifierTab.tsx`

**Modified Files:**
- `Cargo.toml` (workspace member)
- `crates/lr-config/src/types.rs` (add SecretScanningConfig, ClientSecretScanningConfig)
- `crates/lr-config/src/migration.rs` (v22 migration)
- `crates/lr-server/Cargo.toml` (add lr-secret-scanner dep)
- `crates/lr-server/src/state.rs` (add secret_scanner to AppState)
- `crates/lr-server/src/routes/chat.rs` (integrate scanning in pipeline)
- `src-tauri/src/ui/commands.rs` or new `commands_secret_scanning.rs` (Tauri commands)
- `src-tauri/Cargo.toml` (add lr-secret-scanner dep)
- `src-tauri/src/main.rs` (register commands, initialize engine)
- `src/views/optimize-overview/index.tsx` (add card)
- `src/views/optimize-overview/OptimizeDiagram.tsx` (add node)
- `src/views/optimize-overview/constants.ts` (add color)
- `src/types/tauri-commands.ts` (add TS types)
- `website/src/components/demo/TauriMockSetup.ts` (add mock handlers)
- `src/App.tsx` or router config (add route)

### 8. Key Design Decisions

1. **Regex-first, ML-optional**: Regex+entropy provides fast, reliable detection with no external dependencies. ML model is a precision add-on for users who want fewer false positives.

2. **Betterleaks patterns embedded, not fetched**: Ship a snapshot of betterleaks patterns as a compiled-in TOML constant. No network dependency. Can update with app releases. Future: optional auto-update.

3. **Provider-routed ML** (same as guardrails): The ML verifier routes through existing LLM providers (Ollama, etc.) rather than embedding a model. This keeps the binary small and leverages existing infrastructure.

4. **Parallel execution**: Secret scanning runs in parallel with guardrails and compression — no additional latency in the happy path (no secrets found).

5. **Action hierarchy**: Global default → per-category override → per-rule override → per-client override. Most specific wins.

6. **Scan position**: Before guardrails in the pipeline. Secrets should be caught before any other processing to minimize exposure.

7. **Masking strategy**: Replace matched text with `[REDACTED:category]` (e.g., `[REDACTED:aws_key]`). Preserves message structure while removing the secret. The original is never logged.

8. **No secret storage**: Findings are ephemeral — used only for action resolution and event emission. Matched secret text is never persisted to disk or logs.
