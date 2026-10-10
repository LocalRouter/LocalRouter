# Subscription & usage-limit tracking

## Goal
Show how much of each subscription (Claude Pro/Max, ChatGPT Plus/Pro, GitHub
Copilot) and each API account's limits are used, where that usage is heading
before the window resets, and what it is worth in money — on the Dashboard and
in the menu bar. Inspired by the Claude Code status line
(`~/.claude/statusline-command.sh`): 5h + weekly windows, pace-based color,
per-unit usage sparkline, projected end-of-window usage.

## Data sources (cheapest / most private first)
1. **Passive response headers** on traffic LocalRouter carries:
   - HTTPS inspection proxy (`lr-proxy/src/transport.rs::proxy_request`): the
     request's auth says subscription vs API (Anthropic `Bearer sk-ant-oat…` /
     `oauth-` beta = Claude subscription, `x-api-key` = API; `chatgpt.com` =
     ChatGPT subscription, `api.openai.com` = API). Response headers carry
     `anthropic-ratelimit-unified-{5h,7d,…}-{utilization,reset}` + `-status`,
     `x-codex-{primary,secondary}-{used-percent,window-minutes,reset-at}`,
     `x-codex-credits-*`, and generic `x-ratelimit-*` / `anthropic-ratelimit-*`.
     Codex websockets: the 101 handshake headers and in-stream
     `codex.rate_limits` events.
   - Gateway providers: a `reqwest_middleware` observer in
     `lr-providers/src/http_client.rs`.
2. **Passive usage bodies**: Claude Code's `/api/oauth/usage` and
   `/api/oauth/profile`, Codex's `/backend-api/wham/usage` through the proxy.
3. **Active polling** of providers connected in LocalRouter (setting, default
   on): ChatGPT Plus OAuth (`wham/usage`), Copilot (`copilot_internal/user`),
   provider credits APIs (OpenRouter).
4. **CLI logins** (setting, default off — reads another app's credentials):
   Claude Code keychain / `~/.claude/.credentials.json` → `/api/oauth/usage`
   + plan tier; Codex `~/.codex/auth.json` → `wham/usage`. Never stored or
   refreshed.
5. **Ledger**: API-equivalent cost + tokens per account in 15-minute buckets
   from proxy and gateway traffic.

Polling is traffic-driven: every `poll_interval_secs` (5 min) while the
account saw requests in the last 10 min, `idle_poll_interval_secs` (1 h)
otherwise; a fresh passive reading answers the poll. Failures back off
5 → 10 → 20 min, capped at 1 h, with jitter; `Retry-After` (seconds or
HTTP-date) is honoured; a manual refresh bypasses it. Polls never use a proxy.

## Model (`crates/lr-usage`)
- Account id = `<provider>:<subscription|api>`.
- Per account: plan (raw + label + inferred monthly price, user-overridable),
  windows with samples and past-window peaks, request/token quotas, credits,
  unified status, sources, ledger. Persisted to
  `<config_dir>/usage_limits_state.json` (debounced).
- Derived per window: elapsed fraction (even-pace tick), projected % at reset,
  ETA to the limit, pace status, per-slot usage (hour / day), API-equivalent
  cost in the window, share of the plan price.

## Settings
- `usage_tracking`: `enabled` (true), `poll_provider_apis` (true),
  `read_cli_logins` (false), `poll_interval_secs` (300),
  `idle_poll_interval_secs` (3600), `plans` overrides, `hidden_accounts`,
  `tray_items_added` (windows auto-added to the tray once).
- Settings → Usage tab: toggles, intervals, poll status, plan/price overrides,
  hide / forget accounts.

## Menu bar
Usage windows are tray stats items (`TraySource::Usage { account, window }`)
next to All / clients / providers / models in Appearance → Tray Stats, with
the live preview. A newly seen subscription's headline window (weekly, else
monthly) is added automatically, once. Display is Graph or Number: request
items draw the sparkline / their figure, usage windows draw the gauge (upright
or flat per the Labels setting) / their %. `usage_bar` (the retired relative
request gauge) deserializes as Graph. Tray menu and tooltip get one line per
usage window.

## Dashboard
Compact "Usage limits" strip between Traffic and the monitor: one card per
account, one line per window (meter with even-pace tick and projection shade,
%, time to reset), details and the per-slot sparkline in tooltips, value line
(API value 30d vs. plan, or API spend).

## Steps
1. [x] `lr-config`: `UsageTrackingConfig`, `TraySource::Usage`, Graph/Number.
2. [x] `lr-usage` crate: types, header/body parsers, classifier, tracker,
       projections, plan pricing, fetchers, CLI logins, persistence, tests.
3. [x] Proxy capture (headers, usage bodies, websocket, ledger).
4. [x] Gateway capture (http_client observer, finalize ledger).
5. [x] App wiring: tracker, traffic-driven poller with back-off, commands,
       `usage-limits-changed`, tray items + menu lines.
6. [x] Frontend: types, dashboard strip, Usage tab, Tray Stats items, mocks.
7. [x] Local debug run against real Claude / ChatGPT logins.

## Final steps
1. [x] Plan review against the implementation.
2. [x] Test coverage review (parsers, classifier, projections, tracker,
       scheduler, tray items, frontend helpers).
3. [x] Bug hunt.
4. [ ] Commit and push.
