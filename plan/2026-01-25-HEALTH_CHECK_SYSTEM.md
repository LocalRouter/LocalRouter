# Centralized Health Check System - Implementation Plan

## Overview
Reorganize the health check system to be backend-driven with caching, aggregate status display in sidebar, and periodic/failure-triggered checks.

## Requirements Summary
1. **Aggregate health dot in sidebar** - RED (server down), GREEN (all healthy), YELLOW (issues)
2. **Hover tooltip** - Shows server, providers, and MCPs with status
3. **Backend caching** - Health results cached, frontend pulls cache
4. **Configurable check mode** - Periodic (10min) or on-failure only
5. **Failure-triggered re-checks** - Provider/MCP errors trigger health re-check
6. **Manual refresh** - Individual item refresh + main dot for full refresh
7. **Tray icon status dot** - Overlay small colored dot on dynamic graph icon

---

## Files to Modify

### Backend (Rust)
| File | Changes |
|------|---------|
| `src-tauri/src/config/mod.rs` | Add `HealthCheckConfig` with mode and interval |
| `src-tauri/src/providers/health_cache.rs` | **NEW** - Centralized cache manager |
| `src-tauri/src/providers/mod.rs` | Export health_cache module |
| `src-tauri/src/ui/commands.rs` | Add `get_health_cache`, `refresh_all_health` commands |
| `src-tauri/src/server/routes/chat.rs` | Add failure-triggered provider re-check |
| `src-tauri/src/server/routes/mcp.rs` | Add failure-triggered MCP re-check |
| `src-tauri/src/server/state.rs` | Add `health_cache` field to AppState |
| `src-tauri/src/main.rs` | Initialize cache, start periodic task, register commands |
| `src-tauri/src/ui/tray_graph.rs` | Add status dot overlay to graph generation |
| `src-tauri/src/ui/tray.rs` | Pass health status to graph generation |

### Frontend (React)
| File | Changes |
|------|---------|
| `src/components/layout/sidebar.tsx` | Enhanced status dot with tooltip and click handler |

---

## Implementation Details

### 1. Configuration Schema
```rust
// In src-tauri/src/config/mod.rs
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HealthCheckMode {
    #[default]
    Periodic,    // Check every interval_secs
    OnFailure,   // Only check when requests fail
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthCheckConfig {
    #[serde(default)]
    pub mode: HealthCheckMode,
    #[serde(default = "default_interval")]  // 600 = 10 minutes
    pub interval_secs: u64,
    #[serde(default = "default_timeout")]   // 5 seconds
    pub timeout_secs: u64,
}
```

Add to AppConfig:
```rust
#[serde(default)]
pub health_check: HealthCheckConfig,
```

### 2. Health Cache Manager (NEW FILE)
Create `src-tauri/src/providers/health_cache.rs`:

```rust
pub enum AggregateHealthStatus {
    Red,    // Server down
    Green,  // All healthy
    Yellow, // Some issues
}

pub struct ItemHealth {
    pub name: String,
    pub status: String,  // healthy, degraded, unhealthy, ready, pending
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    pub last_checked: DateTime<Utc>,
}

pub struct HealthCacheState {
    pub server_running: bool,
    pub server_port: Option<u16>,
    pub providers: HashMap<String, ItemHealth>,
    pub mcp_servers: HashMap<String, ItemHealth>,
    pub last_refresh: Option<DateTime<Utc>>,
}

pub struct HealthCacheManager {
    cache: Arc<RwLock<HealthCacheState>>,
    app_handle: Arc<RwLock<Option<AppHandle>>>,
}
```

Key methods:
- `get()` - Return current cached state
- `update_server_status(running, port)`
- `update_provider(name, health)`
- `update_mcp_server(id, health)`
- `init_providers(names)` / `init_mcp_servers(configs)` - Set initial "pending" state
- `aggregate_status()` - Calculate Red/Yellow/Green

Emits `"health-status-changed"` Tauri event on every update.

### 3. New Tauri Commands
```rust
#[tauri::command]
pub async fn get_health_cache(...) -> Result<HealthCacheState, String>

#[tauri::command]
pub async fn refresh_all_health(...) -> Result<(), String>
```

### 4. Failure-Triggered Health Checks
In `routes/chat.rs` error handling:
```rust
if let Err(e) = provider.complete(&request).await {
    // Spawn async task to re-check this provider
    tokio::spawn(async move {
        let health = provider.health_check().await;
        health_cache.update_provider(name, health);
    });
    return Err(...)
}
```

Same pattern in `routes/mcp.rs` for MCP server failures.

### 5. Periodic Health Check Task
In `main.rs` setup:
```rust
if config.health_check.mode == HealthCheckMode::Periodic {
    let interval = Duration::from_secs(config.health_check.interval_secs);
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(interval);
        loop {
            timer.tick().await;
            // Check all providers and MCPs, update cache
        }
    });
}
```

### 6. Sidebar Status Dot Enhancement
Update `src/components/layout/sidebar.tsx`:

- Replace `serverStatus` with `healthState: HealthCacheState`
- Calculate aggregate status: RED/YELLOW/GREEN
- Enhanced tooltip showing:
  - Server status (with port)
  - Divider
  - LLM Providers list (name + status dot)
  - Divider
  - MCP Servers list (name + status dot)
- Click handler calls `refresh_all_health`

### 7. Tray Icon Status Dot Overlay
Modify `src-tauri/src/ui/tray_graph.rs`:

Add parameter to `generate_graph`:
```rust
pub fn generate_graph(
    data_points: &[DataPoint],
    config: &GraphConfig,
    health_status: Option<AggregateHealthStatus>,  // NEW
) -> Option<Vec<u8>>
```

Draw a 5x5 filled circle in top-left corner (position ~4,4):
- Green: `Rgba([34, 197, 94, 255])`
- Yellow: `Rgba([234, 179, 8, 255])`
- Red: `Rgba([239, 68, 68, 255])`

For macOS template mode, draw with actual colors (not template-inverted) by using a non-template overlay or drawing the dot after the template processing.

Update `tray.rs` `update_tray_graph_impl` to pass health status from cache.

---

## Event Flow

```
TRIGGERS                          BACKEND                           FRONTEND
─────────────────────────────────────────────────────────────────────────────
1. Periodic timer ─────┐
2. Request failure ────┼─► HealthCacheManager ─► emit "health-status-changed"
3. Manual click ───────┤   (updates cache)              │
4. App startup ────────┘                                ▼
                                                   sidebar.tsx
                                                   (listens, updates UI)
```

---

## Implementation Order

### Phase 1: Configuration (Backend)
1. Add `HealthCheckMode` enum and `HealthCheckConfig` struct to `config/mod.rs`
2. Add `health_check` field to `AppConfig` with defaults

### Phase 2: Cache Manager (Backend)
3. Create `providers/health_cache.rs` with `HealthCacheManager`
4. Export from `providers/mod.rs`
5. Add to `server/state.rs` AppState

### Phase 3: Initialization (Backend)
6. Initialize `HealthCacheManager` in `main.rs`
7. Add to Tauri managed state
8. Initialize provider/MCP lists with "pending" status
9. Start periodic task if configured

### Phase 4: Commands (Backend)
10. Add `get_health_cache` command
11. Add `refresh_all_health` command
12. Register commands in `main.rs`

### Phase 5: Failure Triggers (Backend)
13. Add failure-triggered re-check in `routes/chat.rs`
14. Add failure-triggered re-check in `routes/mcp.rs`

### Phase 6: Tray Icon (Backend)
15. Modify `tray_graph.rs` to accept health status and draw overlay dot
16. Update `tray.rs` to pass health status to graph generation

### Phase 7: Frontend
17. Update `sidebar.tsx`:
    - Fetch initial health state via `get_health_cache`
    - Subscribe to `health-status-changed` event
    - Implement aggregate status calculation
    - Implement enhanced tooltip with sections
    - Add click handler for `refresh_all_health`

---

## Verification

1. **Configuration**: Check `~/.localrouter/settings.yaml` shows `health_check` section
2. **Cache initialization**: On app start, all providers/MCPs show "pending" status
3. **Periodic checks**: With `mode: periodic`, health updates every 10 minutes
4. **Failure triggers**: Force a provider error, verify health re-check occurs
5. **Manual refresh**: Click sidebar dot, verify all items refresh
6. **Aggregate status**:
   - Stop server → RED dot
   - All healthy → GREEN dot
   - One provider down → YELLOW dot
7. **Tooltip**: Hover shows server, providers, MCPs with correct statuses
8. **Tray icon**: Dynamic graph shows status dot in top-left corner
9. **Individual refresh**: Provider/MCP detail page refresh button works

---

## Settings UI (Optional Enhancement)
Add health check settings to Settings > Server tab:
- Mode dropdown: "Periodic" / "On Failure Only"
- Interval input: seconds (only shown for Periodic mode)
