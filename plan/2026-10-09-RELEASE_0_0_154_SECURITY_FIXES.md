# LocalRouter 0.0.154 release: security review fixes

Nine PRs (#23–#31) from the 2026-10-09 security review of the repository,
merged into master with `--no-ff` merge commits, validated locally, run
locally, and released.

## Progress / todo
- [x] Review each PR for user impact, compatibility with existing deployments
  and how to verify it in production (notes below).
- [x] Merge #23 ci-workflow-injection, #24 docker-appimage-checksum,
  #25 markdown-link-navigation, #26 npm-audit-updates, #27 deps-rustsec,
  #28 gemini-key-header, #29 guardrails-fail-closed,
  #30 mcp-sse-endpoint-origin, #31 mcp-remote-buffer-caps.
  All merged cleanly; #30/#31 both touch `transport/sse.rs` and #23/#24
  both touch `docker.yml` in disjoint hunks.
- [x] Validate: stable clippy, fmt, full workspace tests (105 suites,
  3759 passed, 0 failed), `npx tsc --noEmit`, `npm run build`,
  `cargo audit` (22 → 6 vulnerability advisories, all in the "not fixable
  here" set), `npm audit --omit=dev` (7 left, build tooling only).
- [x] Push master as c8ef937f (closes #23–#31 as merged); CI run
  37994584095.
- [x] Build and run the app locally (`cargo tauri dev --no-watch`), exercise
  the changed paths against the dev server on 33625 (results below).
- [x] Show the user the local run; CI passed on 363b4660 (run 37997438502).
- [x] Release went ahead on 2026-10-10 with everything on master at
  4cce8f5b. Beyond the security fixes it includes: RouteLLM replaced by
  decision routing policies (d661ab97), dashboard live traffic (f14f8895),
  MCP gateway metrics (65cffbe4), catalog threshold default (cded9c01),
  community PRs/issues #16, #19, #17, #18, #20, #21, #22 (see
  2026-10-10-* plans), subscription usage tracking (12451826), the tray menu
  use-after-free crash fix (2026-10-10-TRAY_MENU_USE_AFTER_FREE.md) and the
  usage polling fixes (2026-10-10-USAGE_POLLING_DEFAULTS.md). Pre-release
  reviews of the unreviewed commits found no config-compat problems; CI
  passed on 4cce8f5b (run 38070102340).
- [x] Release run 38071057465: version bump, all five platform builds,
  GitHub release, snaps, Nix pin and post-release succeeded; v0.0.154 is
  published (not draft/prerelease) and all five `latest.json` platform URLs
  return 200. Docker run 38073290370 succeeded.
- [x] Flatpak (both arches) failed: the AppIndicator modules added for #22
  installed into `/app/lib64` (CMake GNUInstallDirs), so
  libayatana-indicator's pkg-config check could not find ayatana-ido. Fixed
  with `-DCMAKE_INSTALL_LIBDIR=lib` (645ef451); verified by building the
  0.0.154 flatpak for both arches from the fixed manifest on the temporary
  branch `ci/flatpak-verify` (run 38073643277), which also checks
  `libayatana-appindicator3.so.1` is in `/app/lib`. The v0.0.154 release has
  no `.flatpak` bundles; the verified bundles exist as that run's artifacts.
- [x] Sync local master with the release version bump.

## Per-change impact, compatibility and production verification

### #23 CI: no event data in `run:` scripts; pin free-disk-space action
- Impact: none on users. Same inputs produce the same outputs; a malformed
  `version` now fails the Docker workflow early instead of reaching URLs.
- Compatibility: `release.yml` already enforced the semver regex; `docker.yml`
  now enforces the same one. Pre-release tags like `v1.0.0-beta.1` still pass.
- Verify in prod: the 0.0.154 release run and the dispatched Docker run both
  resolve the version and complete.

### #24 Docker: verify AppImage SHA-256
- Impact: none on users of the image; the image content is unchanged.
- Compatibility: build args default to empty, so local `docker build .`
  still works (prints a warning). Asset names match the published pattern
  (`LocalRouter_<ver>_amd64.AppImage`, `..._aarch64.AppImage`, checked on
  v0.0.153). `contents: read` is enough for `gh release download`.
- Verify in prod: the Docker run for 0.0.154 shows the "Compute AppImage
  checksums" step with two 64-hex digests and the build passes `sha256sum -c`.

### #25 UI: markdown links open in the OS browser
- Impact: links in monitor event details and release notes now open in the
  system browser instead of replacing the app window. `javascript:`,
  `file:` etc. render as plain text.
- Compatibility: `shell:allow-open` is already in `capabilities/default.json`;
  the demo site renders the same component (no Tauri IPC needed to render).
- Verify in prod: Settings → Updates release notes link opens the browser;
  a monitor event whose content contains a URL opens the browser on click.

### #26 npm lockfile updates
- Impact: `@modelcontextprotocol/sdk` 1.27 → 1.32 is the only runtime
  dependency the webview ships (`src/lib/mcp-client.ts`, Try It Out MCP
  tab). Website: react-router 7.1 → 7.18.
- Compatibility: no `package.json` range changes; `npm ci`, `tsc` and the
  Vite build pass. `npm audit` drops from 1 critical / 16 high to 7
  (build-only tooling).
- Verify in prod: Try It Out → MCP tab lists tools from the local gateway and
  can call one.

### #27 Rust dependency updates; drop unused `oauth2`
- Impact: none functional. Removes reqwest 0.11 / rustls 0.21 tree; bumps
  `tar`, `rustls`, `rustls-webpki`, `h2`, `aws-lc-sys`, `quinn-proto`,
  `crossbeam-epoch` within semver.
- Compatibility: `tar` is used by `lr-engines` to unpack engine archives;
  its unit tests cover extraction. `rustls`/`rcgen` back the HTTPS proxy and
  every reqwest client.
- Verify in prod: provider health checks (TLS) go green, an engine
  download/extract succeeds, the HTTPS inspection proxy still serves.

### #28 Gemini: API key in `x-goog-api-key` header
- Impact: the key no longer appears in URLs, so transport errors returned
  as 502 bodies / logs / monitor events cannot leak it.
- Compatibility: `x-goog-api-key` is the documented alternative to `?key=`
  for the Generative Language API; all six request sites switched. Config
  shape unchanged, no migration.
- Verify in prod: with a Gemini key configured, provider health is healthy,
  model list loads, a chat and a streaming chat succeed. Point the base URL
  at an unreachable host and confirm the 502 body has no `AIza` string.

### #29 Guardrails fail closed (the one real behaviour change)
- Before: when every safety model errored (provider down, 429, revoked key,
  prompt longer than the guard model's context), the request was treated as
  safe and passed through silently.
- After: each failed model adds a flagged `guardrail_error` pseudo-category
  with the default `Ask` action, so the request takes the normal approval
  path. Partial failures are also not safe.
- Who sees a difference: only clients that actually run guardrails, i.e. a
  safety model is configured and the effective category policy (per-client
  entries merged over the global list) is non-empty and not all-Allow.
  The default install has no category actions, so it is unchanged.
  Resolution order for the pseudo-category is the same as for any flag:
  a `guardrail_error` entry, then `__model:<type>`, then `__global`. So a
  deployment whose "All Categories" default is Notify or Allow keeps
  passing requests (with a monitor notification for Notify); Ask or Block
  defaults, or a policy with only specific categories, now get the popup or
  a block when their guard model fails, instead of a silent pass.
- Escape hatch: `guardrail_error: allow` (or `notify`) in that client's
  category actions. The approval popup's "allow these categories" /
  "block these categories" buttons now persist Custom categories too
  (`flagged_category_key` in `src-tauri/src/ui/commands_clients.rs`
  accepts the `{custom: name}` shape; before, those buttons silently
  dropped every Custom category, so the popup would recur every request).
  The popup labels the flag "guardrail error".
- `/v1/moderations` is unchanged: `translate_to_moderation_result` only maps
  known categories, so a failing guard still reports `flagged: false` there.
- No config migration: the pseudo-category is matched by the existing
  string-keyed `category_actions` entries.
- Verify in prod: on a client with guardrails + a category policy, stop the
  guard provider (or set a wrong key) and send a request: expect the
  approval popup naming "guardrail error" and a monitor guardrail event.
  Re-enable the provider: requests flow again without the popup.

### #30 MCP SSE: same-origin `endpoint` event; no redirects
- Impact: legacy SSE servers whose `endpoint` event names another scheme,
  host or port are ignored (logged at warn, POSTs fall back to the configured
  URL). Redirects are no longer followed by the MCP HTTP clients.
- Compatibility risk: a server configured as `http://localhost:PORT/sse`
  that advertises `http://127.0.0.1:PORT/messages` will stop working (host
  string mismatch), as will a server configured over `http://` that 301s to
  `https://`. The reference SDKs (Python, TypeScript, FastMCP, Supergateway)
  all emit relative endpoints, so this is expected to be rare. The fix is to
  configure the URL the server actually advertises/serves. OAuth discovery
  already used `Policy::none()`.
- Verify in prod: existing remote MCP servers (Streamable HTTP and legacy
  SSE) connect and list tools after upgrade; check the log for
  "Ignoring MCP endpoint event" or 3xx status errors.

### #31 MCP buffer caps (16 MiB)
- Impact: a single SSE event or a single stdio line larger than 16 MiB
  closes the transport (reconnect logic takes over for SSE). The inline POST
  path already had the same cap, so no well-behaved server changes
  behaviour.
- Verify in prod: large tool results (a few MiB) still arrive; nothing to
  migrate.

## Local run (dev server 33625, 2026-10-09)

- Gemini (#28): model list shows the provider's 48 models; non-streaming
  chat returns "pong" with usage; streaming returns content chunks and
  `[DONE]`. Upstream 400, 404 and 503 bodies surfaced as 502s contain neither
  the key nor `key=`.
- Guardrails (#29): with the dev config's four safety models (three on
  Ollama, `llama-guard3:1b` not pulled), every request logs
  "4 models, 3 verdicts, 1 errors, 1 actions": the 404 becomes one flagged
  `guardrail_error` action, and the global `__global: notify` policy lets the
  request through (HTTP 200). Before the fix the error was silent.
- MCP SSE origin pin (#30): a fake legacy SSE server on 3002 advertising
  `endpoint: http://127.0.0.1:3003/steal` is logged as "Ignoring MCP endpoint
  event ... not on the same origin"; POSTs fall back to the configured URL and
  the sink on 3003 received zero requests. server-everything on 3001
  (relative endpoint) resolves to `http://127.0.0.1:3001/message?...`.
- Found while testing, pre-existing since ab55a08e (2026-10-03, 0.0.149):
  a legacy SSE server answering a POST with a plain `202 Accepted` body (the
  TypeScript SDK's `SSEServerTransport`) failed with "No valid JSON found in
  response" instead of waiting for the result on the stream. Fixed in
  `read_inline_response` via `inline_response_json`: a non-JSON, non-SSE body
  means "no inline response". Unit test
  `inline_response_json_ignores_plain_acknowledgements`.
- Dependencies (#26/#27): TLS to Gemini, Mistral and OpenRouter and the
  model-list fetches work on the updated rustls/h2; the webview bundle built
  from the updated lockfile loads.
