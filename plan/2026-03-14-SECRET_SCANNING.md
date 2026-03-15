# Secret Scanning for LLM Calls - Implementation Plan

## Context

Developers often paste code, configs, or logs into LLM conversations. This risks accidentally exfiltrating secrets (API keys, tokens, passwords, connection strings) to third-party LLM providers. Secret scanning intercepts outbound requests before they reach providers, detecting potential secrets via regex patterns + Shannon entropy filtering (with optional ML verification), and either asks the user for approval or notifies them.

This plan updates `plan/2026-03-14-SECRET_SCANNING.md` with simplified action model, interactive popup approval, and clear per-client vs global config boundaries.

---

## Key Design Decisions

1. **Three actions only: Ask / Notify / Off** - No auto-block (always human-in-the-loop), no masking (would confuse LLMs)
2. **One global action** - No per-category overrides, no per-rule overrides. Single action applies to all detections.
3. **Per-client: only action override** (Default/Ask/Notify/Off) - Entropy, ML, custom rules, allowlist are global-only.
4. **Blocks outbound requests** - Scanning happens before the request is sent to the provider. Does NOT scan responses.
5. **Runs synchronously before guardrails** - Regex+entropy is sub-millisecond. No point spawning parallel work that might be wasted if the request is blocked.
6. **Reuses FirewallManager** - Same oneshot channel + popup window pattern as guardrails, auto-routing, model firewall.

---

## 1. Core Types (`crates/lr-secret-scanner/src/types.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SecretScanAction {
    Ask,    // Block request, show popup, wait for user decision
    Notify, // Allow request, show notification
    #[default]
    Off,    // No scanning
}

pub enum SecretCategory {
    CloudProvider, AIService, VersionControl, Database,
    Financial, OAuth, Generic, Custom,
}

pub struct SecretRule {
    pub id: String,
    pub description: String,
    pub compiled_regex: regex::Regex,
    pub secret_group: usize,
    pub entropy_threshold: Option<f32>,
    pub keywords: Vec<String>,
    pub category: SecretCategory,
}

pub struct SecretFinding {
    pub rule_id: String,
    pub rule_description: String,
    pub category: SecretCategory,
    pub message_index: usize,
    pub matched_text: String,        // truncated ~40 chars for popup display
    pub entropy: f32,
    pub ml_confidence: Option<f32>,  // None if ML not run
    pub ml_verified: Option<bool>,
}

pub struct ScanResult {
    pub findings: Vec<SecretFinding>,
    pub scan_duration_ms: u64,
    pub rules_evaluated: usize,
}
```

---

## 2. Configuration

### Global Config (add to `AppConfig` in `crates/lr-config/src/types.rs`)

```rust
pub struct SecretScanningConfig {
    pub action: SecretScanAction,               // default: Off
    pub entropy_threshold: f32,                 // default: 3.5 (global only)
    pub custom_rules: Vec<CustomSecretRule>,     // global only
    pub ml_verifier: Option<SecretMlVerifierConfig>, // global only
    pub scan_system_messages: bool,             // default: false
    pub allowlist: Vec<String>,                 // global only, regex patterns
}

pub struct CustomSecretRule {
    pub id: String,
    pub description: String,
    pub regex: String,
    pub entropy: Option<f32>,
    pub keywords: Vec<String>,
    pub enabled: bool,
}

pub struct SecretMlVerifierConfig {
    pub enabled: bool,
    pub provider_id: String,
    pub model_name: String,
    pub confidence_threshold: f32,  // default: 0.7
}
```

### Per-Client Config (add to `Client` in `crates/lr-config/src/types.rs`)

```rust
pub struct ClientSecretScanningConfig {
    pub action: Option<SecretScanAction>,  // None = inherit global
}
```

This is the ONLY per-client option. Resolution: `client.action.unwrap_or(global.action)`.

### YAML Example

```yaml
secret_scanning:
  action: ask
  entropy_threshold: 3.5
  scan_system_messages: false
  allowlist:
    - "sk-ant-api03-example.*"
  custom_rules:
    - id: internal-api-key
      description: "Internal API key format"
      regex: 'INTERNAL_[A-Za-z0-9]{32}'
      entropy: 3.0
      keywords: ["INTERNAL_"]
      enabled: true

clients:
  - name: "cursor"
    secret_scanning:
      action: notify  # or: ask, off, null (= default)
```

### Config Migration

Bump `CONFIG_VERSION` to next version. No-op migration since all new fields have serde defaults.

---

## 3. Crate: `crates/lr-secret-scanner/`

```
crates/lr-secret-scanner/
├── Cargo.toml
└── src/
    ├── lib.rs              # Public API: SecretScanEngine
    ├── engine.rs           # Orchestrates: keywords -> regex -> entropy -> optional ML
    ├── regex_engine.rs     # RegexSet compiled from builtin + custom rules
    ├── entropy.rs          # Shannon entropy calculator
    ├── patterns/
    │   ├── mod.rs          # Pattern registry
    │   └── builtin.rs      # Embedded betterleaks-derived patterns (const TOML)
    ├── ml_verifier.rs      # Optional ML verification via provider routing
    └── types.rs            # SecretRule, SecretFinding, ScanResult, etc.
```

**SecretScanEngine API:**
```rust
impl SecretScanEngine {
    pub fn new(config: &SecretScanningConfig) -> Result<Self>;
    pub async fn scan(&self, texts: &[ExtractedText]) -> ScanResult;
    pub fn has_rules(&self) -> bool;
}
```

**Scan pipeline:**
1. Keyword pre-filter (aho-corasick) - skip rules whose keywords don't appear
2. RegexSet match on text
3. For each match: compute Shannon entropy, discard if below `entropy_threshold`
4. If ML verifier enabled: run on surviving matches for precision boost
5. Return `ScanResult` with findings

Reuses `lr_guardrails::text_extractor::extract_request_text()` to extract scannable text from the request JSON.

---

## 4. Integration into Request Pipeline

### Position in `chat.rs` (and `completions.rs`)

Secret scanning inserts **after rate limits / client mode checks, before guardrail spawn** (around line 284 in `chat.rs`):

```rust
// ... existing: rate limits, client mode check ...

// === Secret Scanning (blocks outbound request if action=Ask) ===
run_secret_scan_check(&state, client_auth.as_ref().map(|e| &e.0), &request).await?;

// Start guardrail scan in parallel (existing code, line 286)
let guardrail_handle = ...
```

### `run_secret_scan_check` function

```rust
async fn run_secret_scan_check(
    state: &AppState,
    client_ctx: Option<&ClientAuthContext>,
    request: &ChatCompletionRequest,
) -> ApiResult<()> {
    let Some(client_ctx) = client_ctx else { return Ok(()) };
    let config = state.config_manager.get();
    let client = state.client_manager.get_client(&client_ctx.client_id);

    // Resolve effective action: per-client override > global
    let effective_action = client.as_ref()
        .and_then(|c| c.secret_scanning.action.as_ref())
        .unwrap_or(&config.secret_scanning.action);

    if *effective_action == SecretScanAction::Off {
        return Ok(());
    }

    // Check time-based bypass (from "Allow for 1 hour")
    if state.secret_scan_approval_tracker.has_valid_bypass(&client_ctx.client_id) {
        return Ok(());
    }

    // Run the scan
    let scanner = state.secret_scanner.read();
    let Some(scanner) = scanner.as_ref() else { return Ok(()) };
    if !scanner.has_rules() { return Ok(()) }

    let request_json = serde_json::to_value(request).unwrap_or_default();
    let texts = lr_guardrails::text_extractor::extract_request_text(&request_json);
    let result = scanner.scan(&texts).await;

    if result.findings.is_empty() {
        return Ok(());
    }

    match effective_action {
        SecretScanAction::Notify => {
            state.emit_event("secret-scan-notify", &result.findings);
            Ok(())  // Allow request to proceed
        }
        SecretScanAction::Ask => {
            handle_secret_scan_approval(state, client_ctx, request, result).await
        }
        SecretScanAction::Off => Ok(()),
    }
}
```

---

## 5. Approval Flow (Ask Action)

Reuses the existing `FirewallManager` oneshot-channel pattern.

### New fields on `FirewallApprovalSession` and `PendingApprovalInfo`

```rust
pub is_secret_scan_request: bool,
pub secret_scan_details: Option<SecretScanApprovalDetails>,
```

### New struct: `SecretScanApprovalDetails`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretScanApprovalDetails {
    pub findings: Vec<SecretFindingSummary>,
    pub scan_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFindingSummary {
    pub rule_id: String,
    pub rule_description: String,
    pub category: String,
    pub matched_text: String,       // truncated preview
    pub entropy: f32,
    pub ml_confidence: Option<f32>, // None if ML not used
}
```

### New method: `FirewallManager::request_secret_scan_approval`

Follows exact same pattern as `request_guardrail_approval`:
- Creates session with `is_secret_scan_request: true`
- Sets `secret_scan_details`
- No timeout (waits indefinitely like guardrails)
- Calls `request_approval_internal`

### `handle_secret_scan_approval` in `chat.rs`

```rust
async fn handle_secret_scan_approval(
    state: &AppState,
    client_ctx: &ClientAuthContext,
    request: &ChatCompletionRequest,
    result: ScanResult,
) -> ApiResult<()> {
    let client = state.client_manager.get_client(&client_ctx.client_id);
    let client_name = client.map(|c| c.name.clone()).unwrap_or_default();

    let details = SecretScanApprovalDetails {
        findings: result.findings.iter().map(|f| SecretFindingSummary {
            rule_id: f.rule_id.clone(),
            rule_description: f.rule_description.clone(),
            category: format!("{:?}", f.category),
            matched_text: f.matched_text.clone(), // already truncated in engine
            entropy: f.entropy,
            ml_confidence: f.ml_confidence,
        }).collect(),
        scan_duration_ms: result.scan_duration_ms,
    };

    let response = state.mcp_gateway.firewall_manager
        .request_secret_scan_approval(
            client_ctx.client_id.clone(),
            client_name,
            request.model.clone(),
            details,
            /* preview string for tray menu */
        ).await.map_err(|e| ApiErrorResponse::internal_error(...))?;

    match response.action {
        AllowOnce | AllowSession | Allow1Minute | Allow1Hour | AllowPermanent => Ok(()),
        _ => Err(ApiErrorResponse::forbidden("Request blocked: secrets detected")),
    }
}
```

### Popup User Actions → FirewallApprovalAction Mapping

| Popup Button | FirewallApprovalAction | Backend Effect |
|---|---|---|
| **Block** | `Deny` | Return 403 to client |
| **Allow** | `AllowOnce` | Let this request proceed |
| **Allow for 1 hour** | `Allow1Hour` | Add to `SecretScanApprovalTracker`, skip popups for 1hr |
| **Disable Client** (in dropdown) | `DisableClient` | Disable client + deny request |
| **Disable Scan for Client** (in dropdown) | `DenyAlways` | Set client `secret_scanning.action = Some(Off)` + deny request |

### Action handling in `submit_firewall_approval` (commands_clients.rs)

Add `is_secret_scan_request` handling alongside existing `is_guardrail_request`:

- `Allow1Hour` + `is_secret_scan_request` → `state.secret_scan_approval_tracker.add_1_hour_bypass(client_id)`
- `DenyAlways` + `is_secret_scan_request` → set `client.secret_scanning.action = Some(Off)` in config, save
- `DisableClient` → already handled generically (disables client)

---

## 6. Time-Based Tracker

### `SecretScanApprovalTracker` in `state.rs`

Identical structure to `GuardrailApprovalTracker`:

```rust
#[derive(Clone, Default)]
pub struct SecretScanApprovalTracker {
    bypasses: Arc<DashMap<String, Instant>>,
}

impl SecretScanApprovalTracker {
    pub fn new() -> Self { ... }
    pub fn has_valid_bypass(&self, client_id: &str) -> bool { ... }
    pub fn add_1_hour_bypass(&self, client_id: &str) { ... }
    pub fn add_bypass(&self, client_id: &str, duration: Duration) { ... }
    pub fn cleanup_expired(&self) { ... }
}
```

Add to `AppState`:
```rust
pub secret_scan_approval_tracker: Arc<SecretScanApprovalTracker>,
pub secret_scanner: Arc<RwLock<Option<Arc<lr_secret_scanner::SecretScanEngine>>>>,
```

---

## 7. Frontend Changes

### 7a. Popup UI (Firewall Approval)

**`src/components/shared/FirewallApprovalCard.tsx`:**
- Add `"secret_scan"` to request type detection (check `is_secret_scan_request`)
- Header: icon `KeyRound` (lucide), title "Secrets Detected", description "Potential secrets found in outbound request"
- Body: findings table showing for each finding:
  - Rule description (e.g. "AWS Access Key ID")
  - Category badge (e.g. "Cloud Provider")
  - Matched text (truncated, monospace)
  - Entropy value
  - ML confidence (if present, with badge)
- No edit mode (`canEdit` excludes `secret_scan`)
- Action buttons:
  - Left (destructive): **Block** button, with dropdown: "Disable Client", "Disable Scan for Client"
  - Right (green): **Allow** button, with dropdown: "Allow for 1 Hour"

**`src/views/firewall-approval.tsx`:**
- Add `is_secret_scan_request` and `secret_scan_details` to `ApprovalDetails` interface
- Pass findings to `FirewallApprovalCard`

### 7b. Global Settings

**`src/views/settings/secret-scanning-tab.tsx`** (new file):
- Action dropdown: Off / Ask / Notify
- When action != Off, show:
  - Entropy threshold slider (2.0 - 5.0, default 3.5)
  - Scan system messages checkbox
  - Custom rules table (add/edit/delete with regex, keywords, entropy override, enabled toggle)
  - Allowlist textarea (one regex per line)
  - ML Verifier section (enable toggle, provider + model selector, confidence threshold slider)

### 7c. Per-Client Settings

In client edit tabs, add a "Secret Scanning" section (or new tab `secret-scanning-tab.tsx`):
- Single dropdown: Default (shows current global in parens) / Ask / Notify / Off
- No other options

### 7d. TypeScript Types (`src/types/tauri-commands.ts`)

```typescript
export interface SecretScanningConfig {
  action: "ask" | "notify" | "off"
  entropy_threshold: number
  custom_rules: CustomSecretRule[]
  ml_verifier: SecretMlVerifierConfig | null
  scan_system_messages: boolean
  allowlist: string[]
}

export interface ClientSecretScanningConfig {
  action: "ask" | "notify" | "off" | null  // null = default
}

export interface CustomSecretRule {
  id: string
  description: string
  regex: string
  entropy: number | null
  keywords: string[]
  enabled: boolean
}

export interface SecretMlVerifierConfig {
  enabled: boolean
  provider_id: string
  model_name: string
  confidence_threshold: number
}

export interface SecretFindingSummary {
  rule_id: string
  rule_description: string
  category: string
  matched_text: string
  entropy: number
  ml_confidence: number | null
}

export interface SecretScanApprovalDetails {
  findings: SecretFindingSummary[]
  scan_duration_ms: number
}
```

### 7e. Demo Mock (`website/src/components/demo/TauriMockSetup.ts`)

Add mock handlers for:
- `get_secret_scanning_config` → default config
- `update_secret_scanning_config` → no-op
- `get_client_secret_scanning_config` → `{ action: null }`
- `update_client_secret_scanning_config` → no-op
- `test_secret_scan` → mock findings
- `rebuild_secret_scanner` → no-op

---

## 8. Tauri Commands

```rust
// Global config
get_secret_scanning_config() -> SecretScanningConfig
update_secret_scanning_config(config_json: String) -> ()

// Per-client config
get_client_secret_scanning_config(client_id: String) -> ClientSecretScanningConfig
update_client_secret_scanning_config(client_id: String, config_json: String) -> ()

// Testing
test_secret_scan(input: String) -> ScanResult
test_secret_rule(regex: String, test_input: String) -> Vec<TestMatch>

// Engine management
rebuild_secret_scanner() -> ()
```

---

## 9. Implementation Phases

### Phase 1: Core Engine + Config + Pipeline
1. Create `crates/lr-secret-scanner/` with regex engine, entropy, builtin patterns
2. Add `SecretScanningConfig`, `ClientSecretScanningConfig`, `SecretScanAction` to `lr-config/types.rs`
3. Config migration (no-op)
4. Add `SecretScanApprovalTracker` + `secret_scanner` to `AppState`
5. Add `is_secret_scan_request`, `secret_scan_details`, `request_secret_scan_approval` to `FirewallManager`
6. Add `SecretScanApprovalDetails`, `SecretFindingSummary` to `firewall.rs`
7. Integrate `run_secret_scan_check` into `chat.rs` and `completions.rs` (after rate limits, before guardrail spawn)
8. Handle `is_secret_scan_request` in `submit_firewall_approval`
9. Initialize `SecretScanEngine` at startup + on config reload
10. Unit tests for pattern matching, entropy, scan pipeline

### Phase 2: Frontend
1. TypeScript types in `tauri-commands.ts`
2. Tauri commands (get/update config, test scan, rebuild)
3. Firewall approval card: `secret_scan` request type with findings display + action buttons
4. Firewall approval view: handle `is_secret_scan_request` + `secret_scan_details`
5. Settings tab: `secret-scanning-tab.tsx` (global config only)
6. Per-client dropdown in client edit view
7. Demo mock handlers
8. Register commands in `main.rs`

### Phase 3: ML Verifier (Optional, Later)
1. ML verifier integration using provider-routed inference
2. ML config UI section in settings tab
3. Display ML confidence in popup findings

---

## 10. Files to Create/Modify

### New Files
- `crates/lr-secret-scanner/Cargo.toml`
- `crates/lr-secret-scanner/src/lib.rs`
- `crates/lr-secret-scanner/src/engine.rs`
- `crates/lr-secret-scanner/src/regex_engine.rs`
- `crates/lr-secret-scanner/src/entropy.rs`
- `crates/lr-secret-scanner/src/patterns/mod.rs`
- `crates/lr-secret-scanner/src/patterns/builtin.rs`
- `crates/lr-secret-scanner/src/ml_verifier.rs`
- `crates/lr-secret-scanner/src/types.rs`
- `src/views/settings/secret-scanning-tab.tsx`
- `src/views/clients/tabs/secret-scanning-tab.tsx`

### Modified Files
- `Cargo.toml` — add workspace member + dependency
- `crates/lr-config/src/types.rs` — add `SecretScanningConfig`, `ClientSecretScanningConfig`, `SecretScanAction`; add fields to `AppConfig` and `Client`
- `crates/lr-config/src/migration.rs` — bump version, add no-op migration
- `crates/lr-server/src/state.rs` — add `SecretScanApprovalTracker`, add fields to `AppState`
- `crates/lr-server/src/routes/chat.rs` — insert `run_secret_scan_check` + `handle_secret_scan_approval`
- `crates/lr-server/src/routes/completions.rs` — same integration as chat.rs
- `crates/lr-mcp/src/gateway/firewall.rs` — add `is_secret_scan_request`, `secret_scan_details`, `SecretScanApprovalDetails`, `SecretFindingSummary`, `request_secret_scan_approval()`
- `src-tauri/src/ui/commands_clients.rs` — handle `is_secret_scan_request` in `submit_firewall_approval`; add new Tauri commands
- `src-tauri/src/main.rs` — register commands, initialize engine
- `src-tauri/Cargo.toml` — add `lr-secret-scanner` dep
- `crates/lr-server/Cargo.toml` — add `lr-secret-scanner` dep
- `src/components/shared/FirewallApprovalCard.tsx` — add `secret_scan` request type
- `src/views/firewall-approval.tsx` — add `is_secret_scan_request` + `secret_scan_details`
- `src/types/tauri-commands.ts` — add all TS types + command params
- `website/src/components/demo/TauriMockSetup.ts` — add mock handlers

---

## 11. Verification

1. **Unit tests**: Pattern matching against known secret formats, entropy thresholds, allowlist exclusions
2. **Integration test**: Send a chat completion request containing a known AWS key → verify popup appears (Ask mode) or notification emits (Notify mode)
3. **Per-client override**: Set one client to Off, another to Ask → verify one is scanned, other isn't
4. **Time-based bypass**: Click "Allow for 1 hour" → verify subsequent requests skip scanning for that client
5. **Disable scan for client**: Click "Disable Scan for Client" → verify client config updated to Off
6. **Build**: `cargo test && cargo clippy && npx tsc --noEmit`
