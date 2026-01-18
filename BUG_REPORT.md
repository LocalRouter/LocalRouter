# Test Failure Bug Report

**Date**: 2026-01-18
**Total Failures**: 12 tests
**Pass Rate**: 94.1% (320/340 tests)

---

## 1. Config Tests - Port Number Mismatch (4 failures)

### Affected Tests
- `config::tests::test_default_config`
- `config::tests::test_server_config_default`
- `config::storage::tests::test_load_nonexistent_creates_default`
- `config::storage::tests::test_save_creates_backup`

### Root Cause
**File**: `src-tauri/src/config/mod.rs:850-857`

The default port is **conditionally set** based on build mode:
```rust
impl Default for ServerConfig {
    fn default() -> Self {
        // Use different port for development to avoid conflicts
        #[cfg(debug_assertions)]
        let default_port = 33625;  // DEBUG MODE

        #[cfg(not(debug_assertions))]
        let default_port = 3625;   // RELEASE MODE
```

Tests run in **debug mode** (33625), but assert for **release mode** (3625).

### Fix
Update tests to check the correct port based on build configuration:

```rust
#[test]
fn test_default_config() {
    let config = AppConfig::default();
    #[cfg(debug_assertions)]
    assert_eq!(config.server.port, 33625);
    #[cfg(not(debug_assertions))]
    assert_eq!(config.server.port, 3625);
}
```

**Locations**:
- `src-tauri/src/config/mod.rs:1155`
- `src-tauri/src/config/mod.rs:1164`
- `src-tauri/src/config/storage.rs` (similar assertions)

---

## 2. MCP Protocol - Message Type Detection (1 failure)

### Affected Test
- `mcp::protocol::tests::test_message_parsing`

### Root Cause
**File**: `src-tauri/src/mcp/protocol.rs:106-110`

The `JsonRpcMessage` enum uses `#[serde(untagged)]` with this order:
```rust
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),    // ← Matches FIRST
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}
```

**Problem**: With untagged enums, serde tries variants **in order**. Since `JsonRpcRequest` has `id: Option<Value>`, it matches JSON without an "id" field (treating it as `id: None`).

**Test Input**:
```json
{"jsonrpc":"2.0","method":"notify","params":{}}
```

**Expected**: `Notification` (no "id" field)
**Actual**: `Request` (with `id: None`)

### Fix Options

**Option 1**: Reorder enum (Notification before Request)
```rust
#[serde(untagged)]
pub enum JsonRpcMessage {
    Response(JsonRpcResponse),      // Most specific (has "result" or "error")
    Notification(JsonRpcNotification), // No "id" field
    Request(JsonRpcRequest),        // Has "id" field (even if None)
}
```

**Option 2**: Use custom deserializer that checks for "id" field presence
```rust
impl<'de> Deserialize<'de> for JsonRpcMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;

        if value.get("result").is_some() || value.get("error").is_some() {
            Ok(JsonRpcMessage::Response(serde_json::from_value(value)?))
        } else if value.get("id").is_some() {
            Ok(JsonRpcMessage::Request(serde_json::from_value(value)?))
        } else {
            Ok(JsonRpcMessage::Notification(serde_json::from_value(value)?))
        }
    }
}
```

**Recommended**: Option 2 (more explicit and robust)

**Location**: `src-tauri/src/mcp/protocol.rs:106-110`

---

## 3. Monitoring Metrics - Database Not Persisting (5 failures)

### Affected Tests
- `monitoring::metrics::tests::test_four_tier_isolation`
- `monitoring::metrics::tests::test_metrics_collector_get_names`
- `monitoring::metrics::tests::test_metrics_collector_record_failure`
- `monitoring::metrics::tests::test_metrics_collector_record_success`
- `monitoring::storage::tests::test_cleanup_old_data`

### Root Cause
**File**: `src-tauri/src/monitoring/metrics.rs:147-177`

The `record_success_at` method writes to the database, but queries return 0 results.

**Potential Issues**:
1. Database write might be failing silently
2. Query time range might not match written timestamp
3. Transaction not being committed
4. Tempdir cleanup happening before read

### Investigation Needed
Check if `MetricsDatabase::insert_metric` is:
- Actually executing the INSERT
- Committing the transaction
- Handling errors properly

**Location**: `src-tauri/src/monitoring/metrics.rs:177-180`

```rust
for metric_type in metric_types {
    if let Err(e) = self.db.insert_metric(&metric_type, &row) {
        tracing::error!("Failed to insert metric: {}", e);
        // ← ERROR IS LOGGED BUT NOT RETURNED
    }
}
```

### Fix
1. Check `MetricsDatabase::insert_metric` implementation
2. Ensure transaction commit
3. Add explicit flush or sync after insert for tests
4. Return error instead of just logging it

---

## 4. Provider - Token Count Test Data Mismatch (1 failure)

### Affected Test
- `providers::tests::test_token_usage_with_all_details`

### Root Cause
**File**: `src-tauri/src/providers/mod.rs:640-664`

**Simple test data bug**:

```rust
let usage = TokenUsage {
    // ...
    prompt_tokens_details: Some(PromptTokensDetails {
        cached_tokens: Some(600),  // ← SET TO 600
        // ...
    }),
};

// ...
assert_eq!(prompt_details["cached_tokens"], 500);  // ← EXPECTS 500
```

### Fix
Change line 645:
```rust
cached_tokens: Some(600),  // ← BUG
```

To:
```rust
cached_tokens: Some(500),  // ← CORRECT
```

**Location**: `src-tauri/src/providers/mod.rs:645`

---

## 5. Clients - Test Checking Wrong Variable (1 failure)

### Affected Test
- `clients::tests::test_create_client`

### Root Cause
**File**: `src-tauri/src/clients/mod.rs:450-455`

Test confusion about Client structure:
- `client.id` = UUID (e.g., "123e4567-e89b-12d3-a456-426614174000")
- `secret` = API key format (e.g., "lr-abc123...")

**Current (Wrong)**:
```rust
let (client_id, secret, config) = manager.create_client("Test Client".to_string())?;
assert!(client_id.starts_with("lr-"));  // ← WRONG! client_id is UUID
```

**Should Be**:
```rust
let (client_id, secret, config) = manager.create_client("Test Client".to_string())?;
assert!(secret.starts_with("lr-"));  // ← CORRECT! secret is the API key
assert!(client_id.len() == 36);  // UUID format check instead
```

### Fix
Change line 455:
```rust
assert!(client_id.starts_with("lr-"));  // ← BUG
```

To:
```rust
assert!(secret.starts_with("lr-"));  // ← CORRECT
assert!(uuid::Uuid::parse_str(&client_id).is_ok());  // Verify UUID format
```

**Location**: `src-tauri/src/clients/mod.rs:455`

---

## Summary of Fixes Required

| Bug | Type | Difficulty | Files |
|-----|------|-----------|-------|
| Port number tests | Test update | Easy | `config/mod.rs`, `config/storage.rs` |
| MCP message parsing | Deserialization logic | Medium | `mcp/protocol.rs` |
| Metrics not persisting | Database issue | Hard | `monitoring/metrics.rs`, `monitoring/storage.rs` |
| Token count mismatch | Test data | Trivial | `providers/mod.rs:645` |
| Client ID assertion | Test logic | Trivial | `clients/mod.rs:455` |

**Total Estimated Fix Time**: 2-4 hours

---

## Priority Order

1. **Trivial Fixes** (5 minutes):
   - Token count test data (1 line change)
   - Client ID assertion (1 line change)

2. **Easy Fixes** (30 minutes):
   - Port number tests (conditional compilation)

3. **Medium Fixes** (1 hour):
   - MCP message parsing (custom deserializer or reorder enum)

4. **Hard Fixes** (1-2 hours):
   - Metrics database persistence (requires investigation)

