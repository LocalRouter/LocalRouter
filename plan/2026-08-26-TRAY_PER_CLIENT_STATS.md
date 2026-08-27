# Tray Icon: Per-Client Stats

**Status: implemented (2026-08-26).** Deviations from the design below:
- Usage totals use a new granularity-aware query
  (`MetricsDatabase::get_usage_for_type`) instead of `get_aggregated_usage`,
  which sums minute rows *and* their hourly/daily rollups and so over-counts.
- The tray title is always applied (cleared when text is off or the layout is
  Compact) so a layout switch never leaves stale text in the menu bar.
- An Extended layout with every item unchecked falls back to the single
  global pane instead of rendering nothing.
- The real-time callback carries `lr_types::RecordedRequest`
  (`Option<&RecordedRequest>`; `None` for token-less feature events).

## Context

Today the tray icon is either a static logo or one 32×32 sparkline of **global** token
throughput. With several clients (Claude Code, Cursor, …) going through LocalRouter you
can't see from the menu bar *which* client is busy or how much each has used recently.
This feature adds a standalone "Tray Stats" configuration: an ordered list of items — All
requests, chosen **clients** (new clients auto-added), and chosen **LLMs** (provider
instances such as `anthropic`, or individual models such as `anthropic/claude-sonnet-4`) —
each rendered as a labelled panel in the tray icon, with optional numbers beside the icon
and a usage section in the tray menu. Metrics already record per-client
(`llm_key:{id}`), per-provider (`llm_provider:{instance}`) and per-model
(`llm_model:{provider/model_id}`) tiers, so every source type reads from the same store.

It deliberately does **not** try to model Claude/Codex quota systems (session/weekly/model
limits) — it only shows *recent usage over a chosen period*, absolute and relative to the
other items.

## Platform facts (verified in `tray-icon 0.24.1` and desktop-shell sources)

| | Wider-than-square icon | Text beside icon (`set_title`) |
|---|---|---|
| macOS | ✅ height→18pt, width proportional | ✅ |
| Linux GNOME (AppIndicator ext.) | ✅ aspect ≥1.5 special-cased | ✅ |
| Windows | ❌ squashed to square | ❌ no-op |
| Linux KDE | ❌ shrunk to square | ❌ |

- **Extended tier** (macOS, GNOME/Unity/Cinnamon by `XDG_CURRENT_DESKTOP`, or manual override): full feature.
- **Compact tier** (Windows, KDE, unknown): the icon stays exactly as today (single global panel). The **tray-menu usage section still works everywhere**, so Compact users get the numbers there.
- macOS icons are template images → monochrome; no color coding. Panels are identified by their **stacked vertical label**.

---

## What the user sees

### Icon (Extended tier)
One panel per enabled item, left to right in configured order. Each panel = a 4-char label
stacked vertically (letters upright, top-to-bottom) + the sparkline frame + an optional
thin usage bar. Overlay badge (firewall/health/update) stays on the first panel's frame.

```
 A ┌──────────┐▏   C ┌──────────┐▏   C ┌──────────┐▏     1.2M · 24.1k · 9.8k
 L │  ▂▃▅▇▅▃  │█   L │  ▁▁▂▁▁▁  │▎   U │  ▅▇▇▆▇▇  │▌
 L │          │█   A │          │▎   R │          │▌
   └──────────┘█   U └──────────┘    S └──────────┘▌
  label  graph  usage-bar
```
- **Label**: uppercase, max 4 chars, 5×7 pixel font, one letter per row (4×8px = the full 32px height). Default: `ALL` for global; otherwise the first 4 alphanumerics of the item's name uppercased — client `Claude Code` → `CLAU`, provider `anthropic` → `ANTH`, model `openai/gpt-5` → `GPT5` (model id part only). Editable per item. Global toggle to hide labels.
- **Graph**: today's sparkline, per item, shared vertical scale across panels; metric = tokens or requests (setting).
- **Usage bar** (optional): 2px-wide vertical fill right of the frame = this item's usage over the *usage period* ÷ the largest item's usage. With `ALL` enabled that's each client's share of total; without it, relative to the busiest client.
- **Text beside icon** (optional): usage over the period, one number per item in panel order (`Tokens` / `Cost` / `Requests`).

### Tray menu (all platforms)
New section after the Clients block, one line per enabled item, in the same order:
```
────────────────
Usage · last 24h
   ALL      1.2M tok · 843 req · $3.12
   CLAU     24.1k tok · 31 req · $0.42
   ANTH     980k tok · 610 req · $2.71
   GPT5     9.8k tok · 12 req · $0.18
```
Clicking a client line opens that client's tab (existing `open-client-tab` event); a
provider/model line opens the Resources → provider page; `ALL` opens the dashboard.

### Settings → Appearance → "Tray Stats" card (own config, independent of clients)
- Enable graph (existing radio) stays as-is above it.
- **Items**: ordered list with checkbox (enabled), label text box (4 chars), ▲▼ reorder, ✕ remove. `All requests` is a fixed row. An **Add item** dropdown grouped into *Clients* / *Providers* / *Models* (from `list_clients`, `list_provider_instances`, and the model list the dashboard already uses). Switch: *Automatically add new clients* (default on). Providers/models are added manually only — provider lists are long and mostly idle.
- **Show**: ☑ Labels · ☑ Graph · ☐ Usage bar · ☐ Numbers beside icon (hidden on Windows).
- **Graph metric**: Tokens / Requests.
- **Usage period**: 1 hour / 24 hours / 7 days / 30 days. **Usage metric**: Tokens / Cost / Requests (drives the usage bar, the numbers beside the icon, and the menu lines' leading value).
- **Layout** (Linux only): Auto / Extended / Compact.

---

## Implementation

### 1. Config — `crates/lr-config/src/types.rs`
New standalone struct on `UiConfig` (~line 636), all `#[serde(default)]` → no migration,
`CONFIG_VERSION` unchanged. Never rename/remove variants later.
```rust
pub struct TrayStatsConfig {
    pub items: Vec<TrayStatsItem>,                 // default [All]
    pub auto_add_clients: bool,                    // true
    pub show_labels: bool,                         // true
    pub show_graph: bool,                          // true
    pub show_usage_bar: bool,                      // false
    pub show_text: bool,                           // false
    pub graph_metric: TrayGraphMetric,             // Tokens | Requests
    pub usage_metric: TrayUsageMetric,             // Tokens | Cost | Requests
    pub usage_period: TrayUsagePeriod,             // Hour | Day | Week | Month  (default Day)
    pub layout: TrayLayout,                        // Auto | Extended | Compact
}
pub struct TrayStatsItem { pub source: TraySource, pub enabled: bool, pub label: Option<String> }
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TraySource {
    All,
    Client { id: String },          // metrics key llm_key:{id}
    Provider { instance: String },  // metrics key llm_provider:{instance}   (provider instance_name)
    Model { id: String },           // metrics key llm_model:{id}            (dashboard's `{provider_instance}/{model_id}`)
}
impl TraySource { fn metric_type(&self) -> String }  // "llm_global" | "llm_key:…" | "llm_provider:…" | "llm_model:…"
```
`fn effective_label(&self, clients) -> String` (uppercase, alphanumerics only, ≤4 chars; for
`Model` use the part after the last `/`).

Hooks: `ConfigManager::create_client_with_strategy` (`crates/lr-config/src/lib.rs:284`) pushes
`{Client{id}, enabled: auto_add_clients, label: None}` — covers UI/clone/wizard/try-it-out;
`ConfigManager::delete_client` (`lib.rs:350`) removes the item. Render side also skips ids
that no longer exist.

### 2. Per-source real-time feed
- New `lr_types::RecordedRequest { client_id: String, provider: String, model: String, tokens: u64 }`.
- `crates/lr-monitoring/src/metrics.rs:154,188,245` — callback becomes `Fn(Option<&RecordedRequest>)`
  (`None` = token-less feature event); `record_success_at` builds it from
  `metrics.api_key_name` (= client id), `metrics.provider`, `metrics.model`.
- `crates/lr-types/src/lib.rs:16` — `TokenRecorder::record_request(&self, req: &RecordedRequest)`;
  update the single bypass caller `crates/lr-server/src/routes/chat.rs:914` (`auth.api_key_id`,
  `response.provider`, `response.model` are all in scope — clone before they move into `generation_details`).
- `src-tauri/src/main.rs:2352-2359` — forward the struct.

### 3. Usage-over-period data — `crates/lr-monitoring/src/metrics.rs`
Add `get_usage_for_type(metric_type, window: Duration) -> (requests, tokens, cost)` wrapping
`db().get_aggregated_usage` (`storage.rs:300`); called with `TraySource::metric_type()` for each
item. One SQL round-trip per item; auto-picks minute/hour/day granularity by span (existing logic).

### 4. `TrayGraphManager` — `src-tauri/src/ui/tray_graph_manager.rs`
- State: `HashMap<TraySource, SourceState { buckets: Vec<Bucket{tokens, requests}>, accumulated: Bucket }>`.
  `All` accumulates every request; other sources only when enabled.
- `record_request(req)` → `All` + every enabled source it matches (`Client(req.client_id)`,
  `Provider(req.provider)`, `Model(req.model)`), requests += 1.
- Slow mode: `get_global_range` / `get_key_range` / `get_provider_range` / `get_model_range`
  (`metrics.rs:303-365`) by source. Fast/Medium: per-source accumulators. `compute_bucket_shifts`
  applied to all; config change resets all; idle exit when every source is empty.
- Usage cache: `HashMap<SourceKey, (requests, tokens, cost)>` recomputed on each visual tick (≤1/s, only while active) and by a 60 s idle timer so the rolling window still ages.
- `effective_layout()` from `cfg!(target_os)` + `XDG_CURRENT_DESKTOP` + override. Compact → existing single-global render path untouched.
- Apply in one `run_on_main_thread` closure: `set_icon` + template flag (existing), `set_title`
  (extended only; `None` when `show_text` off), `set_tooltip` (same lines as the menu section).
  Hash `(png, title, tooltip)` for the skip-if-unchanged check.
- Menu refresh: call `rebuild_tray_menu` when the formatted usage lines change, throttled to once per 30 s while active and once when activity stops.

### 5. Renderer — `src-tauri/src/ui/tray_graph.rs` + new `tray_font.rs`
- `tray_font.rs`: `const GLYPHS_5X7: [(char, [u8; 7]); 36]` for A–Z 0–9, `draw_glyph(img, x, y, ch, color)`, `draw_vertical_label(img, x, label)` (rows at y = 0, 8, 16, 24).
- Extract `generate_graph` body into `draw_pane(img, x_offset, PaneSpec, config, overlay)`; bar-height scaling (`:583-627`) takes an explicit shared scale.
- `generate_multi_pane(panes: &[PaneSpec], config, overlay) -> PNG`; panel width = (labels ? 6 : 0) + 32 + (usage_bar ? 4 : 0); image = `sum × 32`.
- `PaneSpec { label: Option<String>, bars: Vec<u64>, usage_fill: Option<f32> }`.
- `#[ignore]` `write_test_multi_pane_to_file` next to the existing visual-dump tests.

### 6. Formatting helpers — `src-tauri/src/ui/tray_format.rs`
`compact_number(u64) -> "24.1k" / "1.2M"`, `compact_cost(f64) -> "$0.42"`, `period_label() -> "last 24h"`,
`usage_line(label, usage) -> "CLAU     24.1k tok · 31 req · $0.42"`. Shared by title, tooltip and menu.

### 7. Tray menu — `src-tauri/src/ui/tray_menu.rs`
In `build_tray_menu` (`:82-604`), after the Clients block: separator, disabled header
`Usage · {period}`, one item per enabled source with id `tray_stats_open__{source}` handled
in the menu-event router (emit `open-client-tab` / navigate to dashboard). Mirror in
`website/src/components/demo/MacOSTrayMenu.tsx` (file header mandates sync).

### 8. Tauri commands — `src-tauri/src/ui/commands.rs:1408-1457`
`get_tray_stats_config` / `update_tray_stats_config(config: TrayStatsConfig)` alongside the
existing `get/update_tray_graph_settings`; register in `main.rs:2700`; types in
`src/types/tauri-commands.ts` (~2784); mock in `website/src/components/demo/TauriMockSetup.ts`.

### 9. Settings UI — `src/views/settings/appearance-tab.tsx`
New "Tray Stats" card as described above (Radix checkbox/select/switch already in `src/components/ui`; client rows show `ServiceIcon` by `template_id`, provider rows the provider icon, model rows a plain badge). The "Add item" dropdown reuses the dashboard's client/provider/model lists (`src/views/dashboard/index.tsx:202-204` builds exactly the ids the metrics use). Items whose client/provider no longer exists render greyed with "(removed)" and a ✕. Fix the stale window copy (26 buckets → 26 s / 4 m 20 s / 26 m). Extend `GraphIconPreview` to render N labelled mini panels reflecting the current config.

---

## Defaults I picked (say if you want different)
- Label font 5×7 px, 4 rows → labels are tiny (iStat-Menus size). Rendering the icon at 2× for Retina crispness is a possible follow-up, not in scope.
- Usage bar relative to the largest item (share of total when `ALL` is on).
- Usage period default 24 h; text/usage-bar off by default; labels + graph on.
- Icon capped at 6 panels (extra items still in menu/tooltip).

## Verification
1. `rustup run stable cargo clippy --workspace --all-targets -- -D warnings`; `cargo fmt --all -- --check`; `cargo test -p localrouter-tauri`, `-p lr-monitoring`, `-p lr-config` (old YAML without `tray_stats` still loads).
2. `cargo test -p localrouter-tauri write_test_multi_pane -- --ignored` → inspect `/tmp/*.png`: labels legible, 1/2/3 panels, usage bars, overlay on first panel only.
3. `cargo tauri dev --no-watch` (macOS): enable All + two clients + one provider + one model, `curl` through each client key / model → the matching panels move (a request via client A on model M moves ALL, A, M, and M's provider); numbers beside icon, tooltip, and menu section match; create a client → auto-appears; delete → gone; reorder/rename label → icon updates.
4. Switch usage period → menu header and values change; usage bar proportions follow.
5. Force `layout: compact` → today's single global icon, menu section still present.
6. `npx tsc --noEmit`.
