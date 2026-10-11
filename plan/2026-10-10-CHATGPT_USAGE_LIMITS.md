# ChatGPT subscription usage limits (ChatGPT provider + Codex)

User report: "ChatGPT Plus provider doesn't show Usage Limits. why? add it
in." and "I have both a ChatGPT Pro provider AND a Codex wrapper. I'd expect
both to trigger a usage limit thingy, with deduping so it doesn't show twice
if it's the same account."

## Progress

- [x] Find why no ChatGPT card shows (evidence from the user's own data)
- [x] Fix Codex header parsing (empty secondary slot)
- [x] Key ChatGPT accounts by login, deduplicated across sources, compatible
      with existing ids and state files
- [x] Route the login through every source (gateway, proxy HTTP, proxy
      websocket, proxied usage bodies, both pollers)
- [x] Dashboard: show a seen subscription even before it has limits; tell
      two accounts on the same plan apart
- [x] Tests
- [x] Plan review, test coverage review, bug hunt
- [x] Commit (no push, per the task)

## Findings

Data inspected read-only (no tokens printed): the production config
(`~/.localrouter/settings.yaml`, config v31), its usage state
(`~/.localrouter/usage_limits_state.json`), `~/.codex/.env` (proxy host only),
and Codex's own log database (`~/.codex/logs_2.sqlite`, which records the
response headers of Codex's HTTP calls).

1. **The account exists but is hidden.** The state file has
   `openai:subscription` with plan `prolite` (Pro Lite, from the token's plan
   claim), sources `proxy_headers` and `gateway_headers`, **no windows and no
   requests**. The Dashboard only shows accounts with windows, credits or
   requests (`visibleAccounts`), so the card never appears.
2. **No traffic carrying limits went through LocalRouter since tracking
   started** (2026-10-10 14:33). The "Codex wrapper" is the Codex client:
   `~/.codex/.env` points Codex at LocalRouter's HTTPS proxy
   (`127.0.0.1:3626`). Codex's log shows that since then it only fetched
   `GET chatgpt.com/backend-api/codex/models` (847 calls, no rate-limit
   headers) and ran no turns (its last websocket turn was on 2026-10-09). The
   ChatGPT Plus provider served no chat request (no access-log entries, empty
   ledger); its `gateway_headers` reading is the same model-list fetch. Each
   of those calls only taught the tracker the plan.
3. **Polling is off for this user.** `usage_tracking.poll_excluded_providers`
   contains `ChatGPT Plus` (added by the v31 migration for providers that
   existed at upgrade) and `read_cli_logins` is false, so neither LocalRouter's
   ChatGPT login nor Codex's login is asked for usage. Not re-enabled here (the
   user chose "off for existing installs"); the per-provider switch already
   exists in Settings → Usage → "Ask connected providers" → "ChatGPT Plus".
4. **Header bug that would have shown a wrong 0% once traffic flowed.** Real
   `/backend-api/codex/responses` headers for this Pro Lite account (from
   Codex's log):
   `x-codex-plan-type: prolite`, `x-codex-primary-used-percent: 13`,
   `x-codex-primary-window-minutes: 10080`, `x-codex-primary-reset-at:
   1791985298`, `x-codex-secondary-used-percent: 0`,
   `x-codex-secondary-window-minutes: 0`, `x-codex-secondary-reset-at: ""`.
   Pro Lite's weekly limit is the *primary* window and the secondary slot is
   empty. The parser mapped the empty secondary to `seven_day` as well
   (fallback for an unknown length), so it overwrote the real 13% with 0% and
   a reset of "now" — the card would show a stale 0%. Codex itself drops a
   slot with zero usage, zero length and no reset
   (`codex-rs/codex-api/src/rate_limits.rs`); LocalRouter now does the same,
   in the header, `wham/usage` and `codex.rate_limits` parsers.
5. **All ChatGPT logins merged into one account.** `openai:subscription` was
   the only ChatGPT account id, so two different ChatGPT accounts (e.g.
   LocalRouter signed in to one, Codex to another) overwrote each other's
   windows in one card.

## Decisions

- **Account identity = login.** `AccountRef` gains `login: Option<String>`, a
  10-hex-digit FNV-1a tag of ChatGPT's `chatgpt_account_id` claim (or the
  `ChatGPT-Account-Id` request header). Same account through any source → same
  tag → one card; different accounts → different cards. The raw account id is
  not written to the settings or state file.
- **Compatible ids, no config migration.** The family's first login keeps the
  base id `openai:subscription`; a state-file account without a login (all
  existing ones) is adopted by the first login seen; further logins are
  `openai:subscription:<tag>`. The binding is stored in the state file
  (`login`, optional field). Plan overrides, hidden accounts, tray items and
  `tray_items_added` keep pointing at the same account. A reading that does
  not name its login goes to the base account (or the family's only account).
- **Login through every source:** gateway (`OpenAIProvider::usage_account`
  reads the cached OAuth token's claim; `UsageObserverMiddleware` classifies
  the request's token), proxy HTTP (request token), proxy websocket
  (`codex.rate_limits` events go to the login that opened the connection),
  proxied `wham/usage` bodies (the request's login), both pollers (their
  token).
- **Polling dedupe.** A source is not due while another source (passive, or
  the other login's poll) has a fresh reading for the same account, so
  LocalRouter's ChatGPT login and Codex's login on one account poll it once.
  The poller remembers which account each ChatGPT source last fed.
- **Labels.** The token's email claim is stored as `login_label` (local only)
  and shown in the card tooltip, Settings, and appended to the card title when
  two shown accounts would otherwise have the same title.
- **Dashboard.** A subscription gets a card once seen, with "No limits
  reported yet …" until a request or poll brings them.
- **Privacy.** No new network requests; excluded providers stay excluded.

## What the user toggles

Nothing is needed for passive tracking: the next Codex turn through the proxy
or ChatGPT Plus request through LocalRouter fills the card. To see limits
without traffic, turn on Settings → Usage → "Ask connected providers" →
**ChatGPT Plus** (LocalRouter's ChatGPT login), and/or **"Use Claude Code and
Codex logins"** (Codex's login). Both on the same ChatGPT account give one card
and one poll.

## Tests

- `lr-usage` headers: Pro Lite headers verbatim (one weekly window, plan from
  `x-codex-plan-type`, no credits), slots with any data kept, `-limit-name`
  labels.
- `lr-usage` bodies: empty/absent secondary in `codex.rate_limits` and
  `wham/usage`; `limit_name` fallback.
- `lr-usage` classify/types: account, plan and email claims; header fallback;
  same account → equal refs, different → different; id round trip; tag
  stability.
- `lr-usage` tracker/lib: gateway + proxy + websocket event + ledger for one
  account → one card under `openai:subscription`; two accounts → two cards,
  usage bodies routed by login; legacy state adopted by the first login and
  kept across restart; unnamed readings; other-source freshness.
- `lr-providers`: provider usage account follows the token's account;
  `cached_access_token`.
- `lr-proxy`: websocket rate-limit events go to the connection's login.
- `localrouter` poller: two logins of one account poll once; status rows show
  the resolved account; report takes plan and email from the token.
- Frontend unit: subscriptions awaiting limits shown; duplicate titles told
  apart.

## Final steps

1. Plan review — every decision above is implemented; `observe_usage_body`
   and `observe_stream_event` callers pass the classified account; Settings
   already had the per-provider switch (only the login label was added).
2. Test coverage review — new branches (empty slot, adoption, unnamed login,
   only-account fallback, dedupe, label) each have a test.
3. Bug hunt — see the report; the traffic map is written outside the state
   lock; `account_mut` binds only unbound accounts; a `None` login never
   unbinds.
4. Commit (no push for this task).
