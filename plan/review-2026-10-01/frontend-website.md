# Desktop frontend and website review — 2026-10-01

## Scope, constraints, and coverage

Owned areas: `src/`, `website/`, root frontend build configuration and `package.json`, and `tests/e2e/`. Read `CLAUDE.md` first. The parent agent owns the repository-wide plan and final aggregation. No commits, dependency installation, model downloads, external requests, inference, browser launches, desktop launches, or GPU execution were performed by this reviewer. CPU-only TypeScript compilation, bundling, Node tests, source parsing, and temporary filesystem fixtures were used.

The machine-readable inventory is `frontend-inventory.json`. It records **268 TypeScript/TSX files, 79,230 lines, 254 useEffect calls and 482 direct invoke calls** at inventory time. Every listed file was parsed with the installed TypeScript parser and included in a repository-wide pattern audit for asynchronous work, events/listeners, storage, network calls, unsafe rendering/evaluation, external assets, URL handling, and resource ownership. The parser reported **zero syntax diagnostics**. Desktop and website baselines independently passed `tsc --noEmit`.

This is not a claim that every line received an equally deep manual review. Files marked **D** below received direct semantic review of the entire small module or the relevant behavior and its callers; **S** means structural/pattern/type/build review, with manual inspection of selected matched code where applicable. Large components, data catalogs and demo handlers have many untouched behaviors. Generated Windows XP bundles, sourcemaps, Playwright reports and dependency lockfiles were inventoried as artifacts, not audited as original implementation or edited.

## Implemented improvements

### F01 — Credential isolation when switching Try It Out clients

Files: `src/views/try-it-out/llm-tab/index.tsx`, `src/views/try-it-out/mcp-tab/index.tsx`.

Previously the selected client changed before the asynchronous `get_client_value` response arrived, leaving the previous client's credential usable in the meantime. Responses could also arrive out of order and install a credential for the wrong selection. Credentials are now stored together with their owning client ID; only a matching credential can be used, pending lookups clear the state, and obsolete responses are ignored. The LLM default selection now chooses from enabled clients instead of selecting a disabled first entry.

### F02 — Incremental model refresh ordering and cache races

File: `src/hooks/useIncrementalModels.ts`.

Refresh previously started before asynchronous event listener registration, so fast local/cache responses could disappear before listeners existed. Refresh now waits for registration promises. A delayed cached-model response no longer overwrites newer per-provider results; the merge preserves refreshed providers, including an intentionally empty refreshed list. Effect-local cancellation prevents an old StrictMode lifecycle from writing through a reused mounted ref. The effect now tracks its actual refresh dependencies.

### F03 — Dynamic event names follow hook props

File: `src/hooks/useTauriListener.ts`.

The hook previously only depended on caller-supplied dependencies, leaving it subscribed to an old event name if the event prop changed. Event names now participate in the dependency list; existing safe listener teardown and latest-handler refs are retained.

### F04 — Download startup failures and event handling

File: `src/hooks/useModelDownload.ts`.

An invoke rejection was ignored whenever a separate failure event was configured, even when startup failed before emitting any event. Invoke errors now transition to failed in either configuration, with duplicate failure callbacks suppressed. Event filters and progress normalizers use current refs rather than stale mount-time closures. Progress rejects non-finite values and clamps to 0–100. A successful completion clears stale error text. Lifecycle behavior across arbitrary model identity changes still deserves a mounted integration suite; no download was executed.

### F05 — Monitor snapshot/live-event correctness

Files: `src/views/monitor/hooks/useMonitorEvents.ts`, new `src/views/monitor/monitor-events.ts`.

Initial/filter-change snapshots could overwrite live events, older filter queries could replace newer results, duplicate events could accumulate, and an update newly matching a status filter was never inserted. A bounded merge now gives live updates precedence, deduplicates by ID, applies the active predicate, sorts by sequence, and inserts newly matching updates. An in-flight query records intervening live updates and merges them into its snapshot. Superseded queries and queries invalidated by Clear cannot repopulate the list.

### F06 — Monitor detail request races

File: `src/views/monitor/hooks/useMonitorEvents.ts`.

Selecting B while A was loading could show A's detail under B's highlighted row. Selection now updates its identity ref synchronously, clears the old detail immediately, and applies only the latest matching detail response. Update-driven detail refreshes use the same guard. Clearing/unmounting invalidates pending detail requests; a missing detail clears the panel instead of retaining another event.

### F07 — MCP connection ownership and cleanup

File: `src/lib/mcp-client.ts`.

Connections now use attempt-local SDK client/transport references and a generation counter. Disconnect invalidates and detaches synchronously before waiting for close; a late handshake cannot reset or close a newer connection. Failed handshakes close their resources and expose the failure without leaving stale state. Transport closure resets connection metadata and subscriptions. Resource-read callbacks verify that both the connection and subscription are still current. The already-declared roots capability now has an explicit empty `roots/list` handler, avoiding a capability/handler mismatch.

### F08 — Complete MCP pagination and subscription rollback

File: `src/lib/mcp-client.ts`.

Tools, resources and prompts previously returned only the first page. They now consume every cursor, including an empty-string cursor, and reject repeated cursors instead of looping. Subscription failure rolls back the locally installed callback; an unsubscribe only drops its callback after the server accepts it, avoiding a local/server mismatch on rejection.

### F09 — Remove raw sampling/elicitation payload logging

File: `src/lib/mcp-client.ts`.

Sampling prompts and elicitation inputs no longer get copied wholesale into the developer console. Other existing diagnostic logs remain; this change is not a claim that all logging in the product has been redacted.

### F10 — Responses SSE decoding and connection release

Files: new `src/lib/sse.ts`, `src/views/try-it-out/llm-tab/chat-panel.tsx`.

The handwritten Responses parser only recognized LF framing and retained its reader on consumer errors. A shared async generator now handles LF/CRLF boundaries split across arbitrary chunks, streaming UTF-8, multi-line data and significant whitespace. It ignores comment-only frames, cancels the stream and releases the reader when the consumer ends or throws. Unterminated final frames are not dispatched. CR-only line endings are not implemented.

### F11 — Chat cancellation cannot clear a new request

File: `src/views/try-it-out/llm-tab/chat-panel.tsx`.

After Stop then Send, the old request's `finally` could null out the new abort controller and reset its loading indicator. Each request now retains its own controller and only clears the shared ref if it still owns it. A synchronous controller guard also blocks two sends before React updates loading state.

### F12 — Image preview allocation and speech cleanup

Files: `src/views/try-it-out/llm-tab/images-panel.tsx`, `src/views/try-it-out/llm-tab/speech-panel.tsx`.

Image inputs allocated URLs before applying the 16-image limit, leaking discarded previews; allocations inside state updaters could also run twice under StrictMode. Allocation now happens outside updaters and only for retained files, with refs synchronized during additions/removals/mask replacement. Editing a result at the limit no longer allocates a discarded URL. Speech requests use an abort controller, stop on unmount and check cancellation before creating blob URLs, so a late response cannot create an unreachable URL after cleanup. Generated speech arrays remain user-session history rather than a new retention policy.

### F13 — Theme resilience and system-theme reactivity

Files: `src/hooks/use-theme.ts`, `website/src/hooks/use-theme.ts`.

Unavailable localStorage and corrupted stored values could crash initialization or produce an invalid theme cycle. Both hooks validate the stored enum and tolerate unavailable reads/writes. System-theme changes now update React state as well as the root class, keeping theme-dependent controls consistent.

### F14 — Key/value editor respects externally loaded data

File: `src/components/ui/KeyValueInput.tsx`.

Local rows were initialized once, so changing the controlled resource/config could leave stale keys and secrets displayed. External changes now refresh the rows while echoing the component's own unfinished rows does not erase them. Edits clone the changed row instead of mutating an existing state object. `Object.fromEntries` preserves literal keys such as `__proto__` as data. The inputs also receive accessible names.

### F15 — Website development icon path confinement

Files: `website/vite.config.ts`, new `website/shared-icons.ts`.

The custom `/icons` middleware joined an arbitrary request path onto the public directory and could expose sibling files through traversal. It now accepts flat image filenames, rejects malformed escapes/separators/nulls, resolves real paths and rejects symlinks escaping the icon directory. A disappearing/unreadable file stream ends with an error response rather than emitting an unhandled stream error. Supported MIME mappings now include JPEG and WebP.

### F16 — Website mock event lifecycle

File: `website/src/stubs/tauri-api-event.ts`.

An event queued before `unlisten` still called the removed handler, and two already queued events both called a once-listener. Delivery now verifies that the exact listener entry remains registered. Empty event sets are removed, and cleanup of an old listener does not remove a replacement registration.

### F17 — External-link scheme checks and opener isolation

Files: `src/components/shared/FirewallApprovalCard.tsx`, `website/src/stubs/tauri-plugin-shell.ts`.

The marketplace approval source link was an unvalidated external string passed to window.open, unlike the already validated marketplace browse links. It now uses the existing HTTP/HTTPS validator and isolates the opener. The website shell stub applies the same scheme restriction and opener isolation. URLs are still opened only by the corresponding user action.

### F18 — Standalone CPU regression entry point

Files: `package.json`, new `tests/e2e/unit.config.ts`, `tests/e2e/unit/*.spec.ts`.

Added `npm run test:unit` using the repository's existing Playwright dependency, with a separate configuration that has no app/global setup, browser fixtures or browser install requirement. Tests use pure functions, temporary files, in-memory event delivery, mocked MCP SDK boundaries and Web Streams. The parent agent wired this command into CI.

### F19 — Fixed Vite port matches Tauri's configured dev URL

File: `vite.config.ts`.

Tauri always opens port 1420, but Vite previously selected another port if 1420 was occupied. `strictPort: true` now fails clearly instead of starting a frontend at a URL Tauri will not load. The associated Tauri media CSP and approval-denial changes are detailed in `tauri-ui.md`.

## Verification ledger

- Baseline `npx --no-install tsc --noEmit`: exit 0, no diagnostics.
- Baseline `npx --no-install tsc --noEmit -p website/tsconfig.json`: exit 0, no diagnostics.
- Structural parser inventory: 268 files, 79,230 lines, zero syntax diagnostics.
- Initial 14-test run: 13 passed, one test expectation failed because macOS canonicalizes `/var` to `/private/var`. The production path resolver correctly returned a canonical path. The test now compares real paths.
- Second 14-test run: **14 passed (958ms)**.
- Expanded suite `npm run test:unit`: **17 passed (21.3s)**, with 2 workers and no browser/app setup.
- Intermediate production builds caught an accidental out-of-scope abortController reference in Stop; corrected before final validation.
- Final passing production build results are recorded below.
- `git diff --check` has passed during implementation; final verification repeated before handoff.

The 17 behavioral cases cover monitor status transitions/filtering/order/deduplication/snapshot merge, icon traversal and symlink escapes, mock once/unlisten semantics, URL scheme rejection, MCP complete pagination/repeated cursors/handshake cleanup/reconnect ownership/transport closure, and SSE framing/UTF-8/multiline/reader cleanup. React mounted behavior, actual native-window interactions, provider requests, UI screenshots and GPU behavior were not exercised.

## Areas inspected without a separate patch

- Existing URL helper already correctly rejected non-HTTP(S) schemes; its established validated callers were retained.
- OpenAI wrapper confines configuration to supplied base URL/key and intentionally permits browser context for desktop/local use; no secret is introduced into static website assets by this wrapper itself.
- Radix modal/switch wrappers and information tooltip already clean up tooltip timers and delegate dialog/switch interaction behavior to installed primitives.
- Connection graph activity expiration deletes aged entries and clears its timer; no new timer leak was found in the inspected implementation.
- Static docs/research content uses react-markdown without raw HTML execution. Pattern matches for `eval` and `innerHTML` were mock code samples, not executing calls. No live source `dangerouslySetInnerHTML`, `document.write`, `eval`, `new Function`, or remotely loaded script/image/style literals were found by the selected audit expressions.
- Permission state controls, wizard creation flow, rate-limit editor and server settings received selected semantic review. Full accessibility audits and every rapid-toggle/autosave interleaving remain outside demonstrated coverage.
- Existing desktop E2E setup builds/launches a Tauri application and creates an Ollama provider; it was deliberately not run under the no-GPU/no-app verification constraints. Its use of a fixed home-directory test config and legacy fixture shape remains a candidate for a future isolated E2E overhaul.

## Remaining risks and follow-up evidence needed

1. Several large resource/client/settings components still have independent asynchronous loads and debounced saves. A consistent request-generation pattern plus a mounted test harness would provide broader race coverage than the focused fixes here.
2. Model-download identity changes and events from externally initiated/retried jobs have no explicit job ID in this generic hook; the new error handling cannot prove cross-job ordering without backend contracts and mounted integration tests.
3. MCP WebSocket support exists in the wrapper but the active UI selects SSE. Browser WebSocket authentication behavior and real SSE reconnection were not exercised.
4. Some frontend error branches log/return generic errors; all backend string-error conversions were not rewritten merely for consistency.
5. Persisted monitor filters are parsed as typed JSON without a full runtime enum schema. Malformed manually edited storage deserves additional mounted coverage.
6. Speech/history memory, very large image attachments, huge event payloads and endless SSE streams need a separate product decision on limits; the fixes do not silently impose new user-visible quotas.
7. Generated Windows XP assets are sourced from another repository. Their original source and browser behavior were not reviewed or rebuilt.
8. Browser-only keyboard/focus/accessibility behavior and native OAuth/popup workflows need a future explicitly permitted interactive run.

## File inventory

Legend: **D** = direct semantic review of the relevant implementation/callers; **S** = structural/pattern/type/build coverage. Counts below are the captured inventory, not a claim of every-line manual inspection.

| File | Lines | Coverage |
|---|---:|---|
| `src/lib/sse.ts` | 26 | D |
| `src/views/monitor/monitor-events.ts` | 27 | D |
| `tests/e2e/unit.config.ts` | 11 | D |
| `tests/e2e/unit/frontend-regressions.spec.ts` | 83 | D |
| `tests/e2e/unit/mcp-client.spec.ts` | 98 | D |
| `tests/e2e/unit/sse.spec.ts` | 39 | D |
| `website/shared-icons.ts` | 18 | D |
| `src/App.tsx` | 401 | S |
| `src/components/Logo.tsx` | 29 | S |
| `src/components/McpServerIcon.tsx` | 15 | S |
| `src/components/OAuthSettingsControls.tsx` | 318 | S |
| `src/components/ProviderForm.tsx` | 565 | S |
| `src/components/ProviderIcon.tsx` | 15 | S |
| `src/components/ServiceIcon.tsx` | 366 | S |
| `src/components/add-resource/DisabledOverlay.tsx` | 40 | S |
| `src/components/add-resource/MarketplaceSearchPanel.tsx` | 452 | S |
| `src/components/add-resource/index.ts` | 9 | S |
| `src/components/client/ClientModeSelector.tsx` | 310 | S |
| `src/components/client/ClientTemplates.tsx` | 766 | S |
| `src/components/client/HowToConnect.tsx` | 1897 | S |
| `src/components/client/ProxyAllowedModels.tsx` | 129 | S |
| `src/components/compression/types.ts` | 37 | S |
| `src/components/connection-graph/ConnectionGraph.tsx` | 207 | S |
| `src/components/connection-graph/hooks/useGraphData.ts` | 160 | D |
| `src/components/connection-graph/index.ts` | 4 | S |
| `src/components/connection-graph/nodes/AccessKeyNode.tsx` | 58 | S |
| `src/components/connection-graph/nodes/CodingAgentNode.tsx` | 41 | S |
| `src/components/connection-graph/nodes/EndpointNode.tsx` | 59 | S |
| `src/components/connection-graph/nodes/MarketplaceNode.tsx` | 41 | S |
| `src/components/connection-graph/nodes/McpServerNode.tsx` | 67 | S |
| `src/components/connection-graph/nodes/ProviderNode.tsx` | 67 | S |
| `src/components/connection-graph/nodes/RouterGroupNode.tsx` | 15 | S |
| `src/components/connection-graph/nodes/SkillNode.tsx` | 41 | S |
| `src/components/connection-graph/types.ts` | 171 | S |
| `src/components/connection-graph/utils/buildGraph.ts` | 810 | S |
| `src/components/guardrails/SafetyModelPicker.tsx` | 276 | S |
| `src/components/icons/category-icons.tsx` | 91 | S |
| `src/components/layout/BugReportDialog.tsx` | 174 | S |
| `src/components/layout/app-shell.tsx` | 209 | S |
| `src/components/layout/command-palette.tsx` | 381 | S |
| `src/components/layout/header.tsx` | 102 | S |
| `src/components/layout/index.tsx` | 4 | S |
| `src/components/layout/sidebar.tsx` | 903 | S |
| `src/components/mcp/McpOAuthModal.tsx` | 246 | D |
| `src/components/mcp/McpServerTemplates.tsx` | 530 | S |
| `src/components/permissions/CategoryActionButton.tsx` | 94 | S |
| `src/components/permissions/ClientToolsIndexingTree.tsx` | 159 | S |
| `src/components/permissions/GatewayIndexingTree.tsx` | 162 | S |
| `src/components/permissions/IndexingStateButton.tsx` | 163 | S |
| `src/components/permissions/McpPermissionTree.tsx` | 273 | S |
| `src/components/permissions/ModelsPermissionTree.tsx` | 157 | S |
| `src/components/permissions/PermissionStateButton.tsx` | 96 | D |
| `src/components/permissions/PermissionTreeSelector.tsx` | 284 | S |
| `src/components/permissions/SkillsPermissionTree.tsx` | 203 | S |
| `src/components/permissions/VirtualMcpIndexingTree.tsx` | 125 | S |
| `src/components/permissions/index.ts` | 14 | S |
| `src/components/permissions/types.ts` | 44 | D |
| `src/components/providers/EmbeddedEngineTab.tsx` | 594 | S |
| `src/components/providers/EngineModelsTab.tsx` | 321 | S |
| `src/components/providers/HuggingFaceAccountCard.tsx` | 259 | S |
| `src/components/providers/LocalModelsTab.tsx` | 1101 | S |
| `src/components/routellm/ThresholdSelector.tsx` | 291 | S |
| `src/components/routellm/types.ts` | 40 | S |
| `src/components/shared/ContentStorePreview.tsx` | 384 | D |
| `src/components/shared/ExperimentalBadge.tsx` | 20 | S |
| `src/components/shared/FeatureClientsCard.tsx` | 116 | S |
| `src/components/shared/FirewallApprovalCard.tsx` | 761 | D |
| `src/components/shared/McpToolDisplay.tsx` | 192 | S |
| `src/components/shared/ModelDownloadCard.tsx` | 108 | S |
| `src/components/shared/RefreshModelsButton.tsx` | 70 | S |
| `src/components/shared/SamplePopupButton.tsx` | 32 | S |
| `src/components/shared/SystemOneAnswers.tsx` | 268 | S |
| `src/components/shared/feature-support-matrix.tsx` | 132 | S |
| `src/components/shared/metrics-chart.tsx` | 509 | S |
| `src/components/shared/model-pricing-badge.tsx` | 187 | S |
| `src/components/shared/stats-card.tsx` | 138 | S |
| `src/components/shared/support-level-badge.tsx` | 107 | S |
| `src/components/strategies/RateLimitEditor.tsx` | 248 | D |
| `src/components/strategy/AllowedModelsSelector.tsx` | 357 | S |
| `src/components/strategy/DragThresholdModelSelector.tsx` | 812 | S |
| `src/components/strategy/PrioritizedModelSelector.tsx` | 346 | S |
| `src/components/strategy/StrategyModelConfiguration.tsx` | 793 | S |
| `src/components/strategy/ThreeZoneModelSelector.tsx` | 1254 | S |
| `src/components/strategy/UnifiedModelsSelector.tsx` | 524 | S |
| `src/components/strategy/index.ts` | 21 | S |
| `src/components/ui/Badge.tsx` | 48 | S |
| `src/components/ui/Button.tsx` | 62 | S |
| `src/components/ui/Card.tsx` | 82 | S |
| `src/components/ui/Input.tsx` | 49 | S |
| `src/components/ui/KeyValueInput.tsx` | 108 | D |
| `src/components/ui/Modal.tsx` | 145 | D |
| `src/components/ui/PresetSlider.tsx` | 101 | S |
| `src/components/ui/Select.tsx` | 200 | S |
| `src/components/ui/Slider.tsx` | 25 | S |
| `src/components/ui/Toggle.tsx` | 49 | D |
| `src/components/ui/TriStateButton.tsx` | 85 | S |
| `src/components/ui/alert-dialog.tsx` | 139 | S |
| `src/components/ui/alert.tsx` | 60 | S |
| `src/components/ui/checkbox.tsx` | 36 | S |
| `src/components/ui/collapsible.tsx` | 9 | S |
| `src/components/ui/command.tsx` | 157 | S |
| `src/components/ui/dialog.tsx` | 119 | S |
| `src/components/ui/dropdown-menu.tsx` | 198 | S |
| `src/components/ui/info-tooltip.tsx` | 92 | D |
| `src/components/ui/label.tsx` | 24 | S |
| `src/components/ui/popover.tsx` | 29 | S |
| `src/components/ui/progress.tsx` | 25 | S |
| `src/components/ui/radio-group.tsx` | 42 | S |
| `src/components/ui/resizable.tsx` | 74 | S |
| `src/components/ui/scroll-area.tsx` | 46 | S |
| `src/components/ui/separator.tsx` | 29 | S |
| `src/components/ui/skeleton.tsx` | 15 | S |
| `src/components/ui/sonner.tsx` | 48 | S |
| `src/components/ui/switch.tsx` | 31 | S |
| `src/components/ui/table.tsx` | 117 | S |
| `src/components/ui/tabs.tsx` | 53 | S |
| `src/components/ui/textarea.tsx` | 24 | S |
| `src/components/ui/tooltip.tsx` | 28 | S |
| `src/components/wizard/ClientCreationWizard.tsx` | 289 | D |
| `src/components/wizard/steps/StepNameAndMode.tsx` | 78 | S |
| `src/components/wizard/steps/StepTemplate.tsx` | 44 | S |
| `src/components/wizard/steps/StepWelcome.tsx` | 64 | S |
| `src/constants/features.ts` | 44 | S |
| `src/constants/safety-model-variants.ts` | 57 | S |
| `src/constants/tab-icons.ts` | 45 | S |
| `src/hooks/use-theme.ts` | 82 | D |
| `src/hooks/useIncrementalModels.ts` | 106 | D |
| `src/hooks/useMetricsSubscription.ts` | 25 | D |
| `src/hooks/useModelDownload.ts` | 164 | D |
| `src/hooks/useTauriListener.ts` | 99 | D |
| `src/lib/mcp-client.ts` | 543 | D |
| `src/lib/openai-client.ts` | 19 | D |
| `src/lib/utils.ts` | 6 | S |
| `src/main.tsx` | 26 | S |
| `src/types/systemone.ts` | 93 | D |
| `src/types/tauri-commands.ts` | 4383 | S |
| `src/utils/errors.ts` | 31 | D |
| `src/utils/url.ts` | 16 | D |
| `src/views/catalog-compression/index.tsx` | 1109 | S |
| `src/views/clients/client-detail.tsx` | 296 | S |
| `src/views/clients/index.tsx` | 320 | S |
| `src/views/clients/tabs/coding-agents-tab.tsx` | 191 | S |
| `src/views/clients/tabs/compression-tab.tsx` | 124 | S |
| `src/views/clients/tabs/config-tab.tsx` | 80 | S |
| `src/views/clients/tabs/context-tab.tsx` | 199 | S |
| `src/views/clients/tabs/guardrails-tab.tsx` | 238 | S |
| `src/views/clients/tabs/info-tab.tsx` | 321 | S |
| `src/views/clients/tabs/json-repair-tab.tsx` | 131 | S |
| `src/views/clients/tabs/llm-optimize-tab.tsx` | 37 | S |
| `src/views/clients/tabs/marketplace-tab.tsx` | 82 | S |
| `src/views/clients/tabs/mcp-tab.tsx` | 66 | S |
| `src/views/clients/tabs/memory-tab.tsx` | 174 | S |
| `src/views/clients/tabs/models-tab-legacy.tsx` | 439 | S |
| `src/views/clients/tabs/secret-scanning-tab.tsx` | 274 | S |
| `src/views/clients/tabs/settings-tab.tsx` | 326 | D |
| `src/views/clients/tabs/skills-tab.tsx` | 106 | S |
| `src/views/clients/tabs/unified-models-tab.tsx` | 982 | S |
| `src/views/coding-agents/index.tsx` | 1175 | S |
| `src/views/compression/index.tsx` | 842 | S |
| `src/views/dashboard/index.tsx` | 617 | S |
| `src/views/debug/index.tsx` | 301 | S |
| `src/views/elicitation-form.tsx` | 271 | D |
| `src/views/firewall-approval.tsx` | 978 | D |
| `src/views/guardrails/guardrails-panel.tsx` | 344 | S |
| `src/views/guardrails/index.tsx` | 650 | S |
| `src/views/json-repair/index.tsx` | 516 | S |
| `src/views/marketplace/index.tsx` | 1242 | S |
| `src/views/mcp-servers/index.tsx` | 181 | S |
| `src/views/mcp-servers/mcp-settings-panel.tsx` | 158 | S |
| `src/views/memory/index.tsx` | 586 | S |
| `src/views/memory/sessions-tab.tsx` | 812 | S |
| `src/views/monitor/event-detail.tsx` | 1782 | S |
| `src/views/monitor/event-filters.tsx` | 329 | D |
| `src/views/monitor/event-list.tsx` | 166 | S |
| `src/views/monitor/hooks/useMonitorEvents.ts` | 124 | D |
| `src/views/monitor/index.tsx` | 164 | D |
| `src/views/monitor/try-it-out-panel.tsx` | 125 | S |
| `src/views/optimize-overview/OptimizeDiagram.tsx` | 109 | S |
| `src/views/optimize-overview/index.tsx` | 346 | S |
| `src/views/resources/compatibility-panel.tsx` | 193 | S |
| `src/views/resources/index.tsx` | 176 | S |
| `src/views/resources/mcp-servers-panel.tsx` | 1637 | S |
| `src/views/resources/models-panel.tsx` | 427 | S |
| `src/views/resources/providers-panel.tsx` | 2073 | S |
| `src/views/response-rag/index.tsx` | 546 | S |
| `src/views/sampling-approval.tsx` | 198 | D |
| `src/views/secret-scanning/index.tsx` | 486 | S |
| `src/views/settings/appearance-tab.tsx` | 689 | S |
| `src/views/settings/general-tab.tsx` | 142 | S |
| `src/views/settings/health-checks-tab.tsx` | 66 | S |
| `src/views/settings/index.tsx` | 67 | S |
| `src/views/settings/licenses-tab.tsx` | 110 | S |
| `src/views/settings/logging-tab.tsx` | 722 | S |
| `src/views/settings/server-tab.tsx` | 370 | D |
| `src/views/settings/updates-tab.tsx` | 501 | S |
| `src/views/skills/index.tsx` | 1008 | S |
| `src/views/strong-weak/index.tsx` | 400 | S |
| `src/views/try-it-out/guardrails-tab/index.tsx` | 514 | S |
| `src/views/try-it-out/llm-tab/chat-panel.tsx` | 763 | D |
| `src/views/try-it-out/llm-tab/embeddings-panel.tsx` | 250 | S |
| `src/views/try-it-out/llm-tab/images-panel.tsx` | 560 | D |
| `src/views/try-it-out/llm-tab/index.tsx` | 1069 | D |
| `src/views/try-it-out/llm-tab/speech-panel.tsx` | 294 | D |
| `src/views/try-it-out/llm-tab/systemone-panel.tsx` | 723 | S |
| `src/views/try-it-out/llm-tab/transcribe-panel.tsx` | 393 | S |
| `src/views/try-it-out/mcp-tab/connection-info-panel.tsx` | 203 | S |
| `src/views/try-it-out/mcp-tab/elicitation-panel.tsx` | 486 | S |
| `src/views/try-it-out/mcp-tab/index.tsx` | 968 | D |
| `src/views/try-it-out/mcp-tab/prompts-panel.tsx` | 347 | S |
| `src/views/try-it-out/mcp-tab/resources-panel.tsx` | 379 | S |
| `src/views/try-it-out/mcp-tab/sampling-panel.tsx` | 531 | S |
| `src/views/try-it-out/mcp-tab/tools-panel.tsx` | 426 | S |
| `src/vite-env.d.ts` | 1 | S |
| `tests/e2e/fixtures/test-helpers.ts` | 169 | D |
| `tests/e2e/global-setup.ts` | 116 | D |
| `tests/e2e/global-teardown.ts` | 20 | D |
| `tests/e2e/playwright.config.ts` | 24 | D |
| `tests/e2e/specs/client-name-change.spec.ts` | 146 | S |
| `vite.config.ts` | 70 | D |
| `website/src/App.tsx` | 53 | S |
| `website/src/components/ArchitectureDiagram.tsx` | 442 | S |
| `website/src/components/ElicitationDemo.tsx` | 77 | S |
| `website/src/components/FirewallApprovalDemo.tsx` | 124 | S |
| `website/src/components/Footer.tsx` | 100 | S |
| `website/src/components/FreeTierFallbackDemo.tsx` | 25 | S |
| `website/src/components/GuardrailApprovalDemo.tsx` | 47 | S |
| `website/src/components/Logo.tsx` | 29 | S |
| `website/src/components/McpViaLlmDiagram.tsx` | 185 | S |
| `website/src/components/Navigation.tsx` | 148 | S |
| `website/src/components/SecretScanApprovalDemo.tsx` | 38 | S |
| `website/src/components/demo/DemoBanner.tsx` | 9 | S |
| `website/src/components/demo/LocalRouterDemo.tsx` | 36 | D |
| `website/src/components/demo/MacOSMenuBar.tsx` | 43 | S |
| `website/src/components/demo/MacOSTrayMenu.tsx` | 370 | S |
| `website/src/components/demo/MacOSWindow.tsx` | 38 | S |
| `website/src/components/demo/TauriMockSetup.ts` | 4779 | S |
| `website/src/components/demo/index.ts` | 5 | S |
| `website/src/components/demo/mockData.ts` | 1421 | S |
| `website/src/components/docs/DocEmbeds.tsx` | 59 | S |
| `website/src/components/docs/MarketplaceDemo.tsx` | 113 | S |
| `website/src/components/docs/MarketplaceInstallDemo.tsx` | 30 | S |
| `website/src/components/docs/MetricsDemo.tsx` | 173 | S |
| `website/src/components/docs/ModelRoutingDemo.tsx` | 55 | S |
| `website/src/components/ui/Badge.tsx` | 43 | S |
| `website/src/components/ui/Button.tsx` | 53 | S |
| `website/src/components/ui/Card.tsx` | 78 | S |
| `website/src/components/ui/dropdown-menu.tsx` | 198 | S |
| `website/src/hooks/use-theme.ts` | 82 | D |
| `website/src/lib/utils.ts` | 6 | S |
| `website/src/main.tsx` | 10 | S |
| `website/src/pages/Demo.tsx` | 40 | S |
| `website/src/pages/Docs.tsx` | 827 | S |
| `website/src/pages/Download.tsx` | 154 | D |
| `website/src/pages/Home.tsx` | 2072 | S |
| `website/src/pages/Research.tsx` | 520 | S |
| `website/src/pages/docs-content.ts` | 26 | S |
| `website/src/pages/research-content.ts` | 23 | S |
| `website/src/stubs/openai.ts` | 202 | S |
| `website/src/stubs/tauri-api-core.ts` | 14 | S |
| `website/src/stubs/tauri-api-event.ts` | 74 | D |
| `website/src/stubs/tauri-api-mocks.ts` | 12 | S |
| `website/src/stubs/tauri-api-webviewWindow.ts` | 7 | S |
| `website/src/stubs/tauri-plugin-dialog.ts` | 6 | S |
| `website/src/stubs/tauri-plugin-process.ts` | 3 | S |
| `website/src/stubs/tauri-plugin-shell.ts` | 19 | D |
| `website/src/stubs/tauri-plugin-updater.ts` | 8 | S |
| `website/src/vite-env.d.ts` | 1 | S |
| `website/vite.config.ts` | 147 | D |

### Additional configuration/content/artifact coverage

Root `package.json`, `tsconfig.json`, `tsconfig.node.json`, `tailwind.config.js`, `postcss.config.js`, `index.html`; website package/config/HTML files; `src/index.css`, `website/src/index.css`; website documentation/research Markdown and public assets received configuration or structural/static-pattern review. Lockfiles and generated `website/public/winxp/**`/Playwright HTML reports were left intact. `CLAUDE.md` was read for conventions.

## Completed production builds

Both production builds exited 0:

```text
npm run build
> tsc && vite build
vite v6.4.1 building for production...
✓ 4071 modules transformed.
✓ built in 38.43s
```

Desktop's largest application chunk: `dist/assets/index-D_dHyGdP.js`, 1,217.36 kB / 292.91 kB gzip. Existing vendor chunking is retained.

```text
(cd website && npm run build:check)
> tsc && vite build
vite v6.4.1 building for production...
✓ 6662 modules transformed.
✓ built in 34.86s
```

Website's largest chunks: `App-Cm77ePKB.js`, 1,969.87 kB / 516.83 kB gzip; `index-OwvwanIY.js`, 1,519.29 kB / 442.30 kB gzip. Both builds warn about chunks above 500 kB and seven-month-old Browserslist data. Website also reports that FirewallApprovalDemo and GuardrailApprovalDemo are both static and dynamic imports, so their dynamic references do not isolate them into separate chunks. Those are recorded performance/dependency-maintenance opportunities, not suppressed warnings or new external package updates.

The final approval-window adjustment retains controls on submission errors and permits denial with invalid edits; see U03 in `tauri-ui.md`. A follow-up desktop typecheck is tracked separately after this adjustment.

Final approval-window follow-up `npx --no-install tsc --noEmit`: exit 0, no diagnostics. Final `git diff --check`: exit 0. Final review additionally preserves an existing resource-subscription callback if its replacement is rejected and prevents stale subscribe/unsubscribe responses from modifying a newer MCP connection.

Final regression rerun after these MCP changes: `npm run test:unit` — **17 passed (15.8s)** using 2 workers. Runtime warnings were Node's `module.register()` deprecation and the runner's `NO_COLOR`/`FORCE_COLOR` precedence notice; no test failed. Modified Rust files also passed `rustfmt --edition 2021 --check` in the same verification pass.

Final desktop and website checks after all frontend source edits also exited 0 with no diagnostics: `npx --no-install tsc --noEmit` and `npx --no-install tsc --noEmit -p website/tsconfig.json`.
