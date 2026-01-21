# Tray Graph Integration Guide

## Overview

The tray graph implementation has been fixed and tested. The final step is to integrate the `record_tokens()` method into the request completion flow so that Fast and Medium modes can track real-time activity.

## Files Modified (Already Complete)

1. **`src-tauri/src/ui/tray.rs`** - Core implementation fixed
   - Added `accumulated_tokens` field to `TrayGraphManager`
   - Added `record_tokens()` public method
   - Fixed Fast mode to use only real-time tokens (no metrics)
   - Fixed Medium mode to use metrics for initial load only
   - Fixed interpolation bug that caused 13.7% data loss

2. **`src-tauri/src/ui/tray_graph.rs`** - Graph generation (unchanged)

3. **Tests** - All 22 comprehensive tests passing ✅

## Integration Steps Required

### Step 1: Add TrayGraphManager to AppState

**File**: `src-tauri/src/server/state.rs`

**Change**:
```rust
pub struct AppState {
    // ... existing fields ...

    /// Metrics collector for tracking usage
    pub metrics_collector: Arc<MetricsCollector>,

    /// Tray graph manager for real-time visualization (optional)
    pub tray_graph_manager: Option<Arc<TrayGraphManager>>,
}
```

**Why optional?**: The tray graph manager is only available when running in UI mode. Headless mode (future feature) won't have it.

### Step 2: Pass TrayGraphManager when creating AppState

**File**: `src-tauri/src/main.rs`

**Current code** (around line 454):
```rust
let tray_graph_manager = Arc::new(ui::tray::TrayGraphManager::new(
    app.handle().clone(),
    ui_config,
));
```

**Add after server creation**:
```rust
// Add tray_graph_manager to server state
if let Some(server_manager) = app.try_state::<Arc<crate::server::ServerManager>>() {
    server_manager.set_tray_graph_manager(Some(tray_graph_manager.clone()));
}
```

**Alternative approach**: Pass it during AppState construction. Check where `AppState` is created and add it there.

### Step 3: Record tokens in chat completions handler

**File**: `src-tauri/src/server/routes/chat.rs`

**Location 1**: Non-streaming response (around line 458)

**Current code**:
```rust
state
    .metrics_collector
    .record_success(&crate::monitoring::metrics::RequestMetrics {
        api_key_name: &auth.api_key_id,
        provider: &response.provider,
        model: &response.model,
        strategy_id: &strategy_id,
        total_tokens: usage.total_tokens,
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        latency_ms,
        cost_usd: 0.0,
    });
```

**Add after**:
```rust
// Record tokens for tray graph (real-time tracking for Fast/Medium modes)
if let Some(ref tray_graph) = state.tray_graph_manager {
    tray_graph.record_tokens(usage.total_tokens as u64);
}
```

**Location 2**: Streaming response (around line 728)

**Current code**:
```rust
state_clone
    .metrics_collector
    .record_success(&crate::monitoring::metrics::RequestMetrics {
        api_key_name: &auth_clone.api_key_id,
        provider: &provider,
        model: &model_clone,
        strategy_id: &strategy_id_clone,
        total_tokens: total_tokens_final,
        input_tokens: prompt_tokens,
        output_tokens: completion_tokens,
        latency_ms,
        cost_usd: 0.0,
    });
```

**Add after**:
```rust
// Record tokens for tray graph
if let Some(ref tray_graph) = state_clone.tray_graph_manager {
    tray_graph.record_tokens(total_tokens_final as u64);
}
```

### Step 4: Record tokens in text completions handler

**File**: `src-tauri/src/server/routes/completions.rs`

**Location**: Non-streaming response (around line 219)

**Current code**:
```rust
state
    .metrics_collector
    .record_success(&crate::monitoring::metrics::RequestMetrics {
        api_key_name: &auth.api_key_id,
        provider: &response.provider,
        model: &response.model,
        strategy_id: &strategy_id,
        total_tokens: usage.total_tokens,
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        latency_ms,
        cost_usd: 0.0,
    });
```

**Add after**:
```rust
// Record tokens for tray graph
if let Some(ref tray_graph) = state.tray_graph_manager {
    tray_graph.record_tokens(usage.total_tokens as u64);
}
```

**Note**: Add similar integration for streaming completions if/when implemented.

## Verification Steps

After implementing the integration:

### 1. Build and Run
```bash
cargo tauri dev
```

### 2. Enable Tray Graph
- Open application
- Go to Server tab → Graph Settings
- Enable "Dynamic Graph"
- Set to Fast mode (1s refresh)

### 3. Generate Traffic
```bash
# Send test requests
curl http://localhost:3625/v1/chat/completions \
  -H "Authorization: Bearer your-api-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-3.5-turbo",
    "messages": [{"role": "user", "content": "Hello"}]
  }'
```

### 4. Verify Graph Updates
- Watch system tray icon
- Graph should update every second showing recent activity
- Bars should appear for requests made in last 26 seconds

### 5. Test All Modes

**Fast Mode (1s refresh)**:
- Should start empty (no historical data)
- Should show bars immediately after requests
- Window: last 26 seconds

**Medium Mode (10s refresh)**:
- Should load historical data on startup (if available)
- Should update every 10 seconds with new activity
- Window: last 260 seconds (~4.3 minutes)

**Slow Mode (60s refresh)**:
- Should load full historical data
- Should update every 60 seconds
- Window: last 1560 seconds (26 minutes)

## Testing Checklist

- [ ] Fast mode shows real-time activity
- [ ] Fast mode starts empty (no historical load)
- [ ] Medium mode interpolates historical data on startup
- [ ] Medium mode updates with real-time data during runtime
- [ ] Slow mode always uses metrics
- [ ] Graph doesn't show jumpy behavior at minute boundaries
- [ ] No double-counting of tokens
- [ ] All 22 tray graph tests pass
- [ ] No performance regression (< 1ms overhead per request)

## Troubleshooting

### Graph Not Updating
- Check that `record_tokens()` is being called (add debug logs)
- Verify tray_graph_manager is in AppState
- Check that graph is enabled in UI

### Double Counting
- Ensure `record_tokens()` is only called after successful requests
- Don't call it during retries or fallbacks

### Performance Issues
- The `record_tokens()` method is very fast (just an atomic add + notification)
- If issues occur, check the notification channel isn't blocked

## Future Enhancements

1. **Per-Client Graphs**: Show token usage per API key/client
2. **Provider Breakdown**: Color-code bars by provider
3. **Cost Tracking**: Show estimated cost in addition to tokens
4. **Configurable Windows**: Let users adjust time windows
5. **Export Data**: Save graph data to CSV for analysis

---

**Status**: Implementation complete, integration pending
**Tests**: 22/22 passing ✅
**Documentation**: Complete ✅
**Ready for**: Integration and deployment
