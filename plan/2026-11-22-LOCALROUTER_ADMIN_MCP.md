# LocalRouter MCP

## Implementation checklist

- [x] Inspect the gateway, configuration, provider, routing, firewall, and Monitor paths.
- [x] Research MCP tool discovery, Agent Skills, credentials, and observability precedents.
- [x] Review the tool inventory with Matus before implementation.
- [x] Register the built-in server, add per-client access controls, and wire the setup skills.
- [x] Implement client, provider, model, routing, firewall, and config tools through shared services.
- [x] Implement Monitor and active intercept tools through the current approval lifecycle.
- [x] Complete configuration coverage against the manifest and finish desktop/demo integration (see 2026-11-23-MCP_GATEWAY_FINAL.md, item 9).
- [x] Plan review. Compare the implemented catalog, documented settings, and end-to-end workflows against this plan and fill gaps.
- [x] Test coverage review. Cover authorization, state changes, provider operations, settings parity, and intercept edge cases.
- [x] Bug hunt. Re-read handlers and desktop integration for stale session access, missed runtime updates, secret exposure, lost edits, duplicate writes, and deadlocks.
- [x] Run applicable local checks, start the debug desktop app, and verify the MCP wire flow.
- [ ] Commit and push the validated implementation under the repository workflow.

## Product and access

Display **LocalRouter** in the **MCPs** sidebar and Servers list. Use the same **BUILT-IN** label and server detail navigation as the other built-ins. In **Clients → MCP → MCP Server Permissions**, include a LocalRouter row with an explicit Allow, Ask, or Off setting and expandable capability grants. Tool names use the `lr_` prefix.

Serve the built-in server on `/mcp/localrouter` using the standard Streamable HTTP MCP transport and the existing bearer or OAuth client credential. Isolate this endpoint's sessions and catalog from the unified gateway. Its server detail page offers the live endpoint URL for copying to an MCP client.

Default to Off for existing and new clients. Use an explicit per-client grant independent of the ordinary “All MCP Servers” setting. Offer separate access for configuration reads, configuration writes, provider actions, monitoring metadata, retained payloads, and live intercept. Support restricting access to specific clients and resources. The desktop owns administration grants, including changing an operator's own scope or granting another client administrative access. A newly created client starts without admin access. Check live grants at call time and before committing calls that paused for approval. Access revoked mid-session must take effect without reconnecting.

Send a compact starting guide through the gateway's existing virtual-server instructions. Put the same entry point in the bootstrap tool description for clients that ignore initialization instructions. Example:

> To set up a client, call lr_clients_create and use the returned client ID. To connect a provider, load the providers skill with lr_skills_get. For model selection, fallbacks, or rate limits, load model-routing. For permissions, load firewall. For a problem with a live request, load monitoring or active-intercept.

Bundle versioned `SKILL.md` files with the app. `lr_skills_list` returns names and descriptions. `lr_skills_get` loads one skill or a named bundled reference through the same MCP. Each skill gives the actual tool sequence, example arguments, output to verify, and recovery steps. Include `client-setup`, `providers`, `model-routing`, `firewall`, `mcp-servers`, `monitoring`, `active-intercept`, `privacy-and-security`, `local-models`, and `advanced-settings`. Skills describe workflows and confer no permissions. Use normal MCP discovery for enabled tools and keep the instruction text short.

## Proposed tool inventory

### Discovery and configuration

| Tool | Behavior |
| --- | --- |
| `lr_status` | Show app version, gateway state and endpoints, enabled capabilities, and safe runtime health. |
| `lr_skills_list`, `lr_skills_get` | Discover skills and load one set of instructions or a named reference. |
| `lr_config_list_sections`, `lr_config_schema` | Discover supported sections and field types, defaults, allowed values, credential handling, and live or restart-required behavior. |
| `lr_config_get` | Read one global section or a scoped object's settings with effective values and explicit overrides where supported. Return a revision for edits. |
| `lr_config_preview` | Validate a typed patch or a set of related patches, and return a sanitized diff, affected objects, and runtime or restart effects. |
| `lr_config_apply` | Apply the previewed changes against the same revision. Return persisted and effective state plus any pending runtime work. Require a new preview if the relevant state changed. |

### Clients

| Tool | Behavior |
| --- | --- |
| `lr_clients_list`, `lr_clients_get` | List or inspect clients by stable ID, including their LLM/MCP modes, enabled state, strategy, and setup status. |
| `lr_clients_create` | Create a client and choose an existing strategy or create its default strategy. Set LLM/MCP modes and optional template. Return client and strategy IDs and the connection setup step. |
| `lr_clients_update`, `lr_clients_delete` | Change client name, modes, strategy, enabled state, template, or config-sync options, or remove the client after reviewing impacts. |
| `lr_clients_connection` | Show endpoints and integration instructions. Open the existing desktop connection setup when a credential or local client config is required. |
| `lr_clients_credentials` | Initiate credential rotation or revocation through the existing keychain-backed flow and report completion and affected sessions. |

### Providers and models

| Tool | Behavior |
| --- | --- |
| `lr_provider_types_list` | List supported provider types, auth methods, setup requirements, and supported API families. |
| `lr_providers_list`, `lr_providers_get` | List configured instances or inspect one instance, including enablement, safe settings, credential state, and cached health. |
| `lr_providers_create`, `lr_providers_update`, `lr_providers_delete` | Add and configure provider instances, including supported custom endpoints and provider settings. Report strategy references and changes to the live registry. |
| `lr_providers_credentials` | Start provider API-key setup, OAuth sign-in, or disconnect through a user-facing flow. Report progress and configured state. |
| `lr_providers_check` | Run the existing connectivity/auth health check for a selected instance and return latency, result, and sanitized error details. Describe whether it makes an upstream request. |
| `lr_providers_test_model` | Run a small, explicit test against the selected instance and model. Use a bounded synthetic input by default and report endpoint, latency, result, usage, and cost when available. Treat live inference as a potentially billable action. |
| `lr_models_list` | List every page of the catalog across providers or for one provider. Filter by capability and availability. Identify cache age, source, failures per provider, and the next cursor. |
| `lr_models_get`, `lr_models_refresh` | Inspect a provider/model pair's context limits, pricing, capabilities, and availability, or refresh the catalog for selected providers and report which succeeded. |
| `lr_models_configure` | Set supported managed-model options and pricing overrides using the existing stores and validation. |
| `lr_local_models_list`, `lr_local_models_manage` | Inspect the installed library, engines, and download state. Start supported download/import/load/unload/cancel/remove actions with typed arguments and progress. |

### Routing and firewall

| Tool | Behavior |
| --- | --- |
| `lr_routing_strategies_list`, `lr_routing_strategies_get`, `lr_routing_strategies_manage` | Inspect, create, update, or delete strategies. Cover model selection, auto-routing, fallbacks, rate limits, free-tier settings, and model permissions. Show which clients share the strategy before an edit. |
| `lr_routing_profiles_list`, `lr_routing_profiles_get`, `lr_routing_profiles_manage` | Inspect and manage profiles, named routes, route versions, and client/strategy bindings through the existing routing-profile operations. |
| `lr_routing_preview` | Explain candidate models, exclusions, profile selection, fallback order, and limits for a client and request. Identify stages requiring real model execution. Preview itself makes no inference call. |
| `lr_firewall_get` | Inspect effective Allow/Ask/Off rules and inheritance for a client and a model, MCP server/tool/resource/prompt, skill, or other supported capability. Surface shared strategy scope. |
| `lr_firewall_preview`, `lr_firewall_apply` | Preview and apply typed rule changes, including resetting a rule to inherit. Show which clients and resources would gain or lose access. Apply through the same policy paths that reevaluate pending requests and refresh active sessions. |

### Upstream MCP, skills, and related resources

| Tool | Behavior |
| --- | --- |
| `lr_mcp_servers_list`, `lr_mcp_servers_get`, `lr_mcp_servers_manage` | Inspect built-in and configured servers and manage external server setup, authentication, enablement, and lifecycle through existing services. Expose transport-specific schemas. |
| `lr_mcp_servers_check`, `lr_mcp_servers_capabilities` | Check connectivity and list a server's tools, resources, prompts, and push capabilities. Explicit connection checks can start configured infrastructure subject to the desktop's existing controls. |
| `lr_skill_sources_manage`, `lr_installed_skills_list`, `lr_installed_skills_manage` | Inspect and configure skill sources and installed skills. Use the existing install review for external assets. |
| `lr_accounts_manage` | Inspect and configure subscription and usage-tracking accounts, delegating sign-in to the relevant desktop/browser flow. |

### Monitoring and active intercept

| Tool | Behavior |
| --- | --- |
| `lr_monitor_stats`, `lr_monitor_metrics` | Get retained-event counts and capacity and query request volume, errors, latency, tokens, and spend by client, provider, or model. |
| `lr_monitor_events_list`, `lr_monitor_event_get` | Search by time, client, category, status, model/provider, or correlation ID. Fetch an event's metadata and routing/error details by ID. Request/response payloads require separate access and follow the original capture policy. Use stable pagination and indicate gaps from retention. |
| `lr_monitor_events_wait` | Wait briefly for new matching events after a cursor. Reuse existing notification infrastructure where appropriate and return after a bounded interval. |
| `lr_monitor_clear` | Clear retained events with explicit confirmation and report what was removed. |
| `lr_intercept_get`, `lr_intercept_start`, `lr_intercept_stop` | View, arm, and stop the same transient rule shown in Monitor. Require target clients, supported categories, and a bounded duration for MCP-created rules. Return rule ID and expiry. Check the ID before stopping or replacing the rule. |
| `lr_intercept_list_pending`, `lr_intercept_get_pending` | List held requests with category, deadline, and supported decisions. Fetch a privacy-filtered preview when authorized. Distinguish Monitor interceptions from ordinary approvals and omit the operator's own admin calls. |
| `lr_intercept_resolve` | Submit a one-shot allow or deny by request ID through the existing approval service, with desktop confirmation for release. Report stale, expired, or already-answered requests. Guardrail, secret scan, and other specialized approval flows retain their own decision semantics. |

## Configuration coverage

Use a single explicit, typed settings manifest. Every supported field should be discoverable as a readable and writable setting, an existing operational action, a user-facing credential flow, or application-owned state with a reason. Use per-section validation and project types for config patches. A broad YAML or arbitrary filesystem editor would bypass service side effects and runtime validation.

| Area | Coverage to verify |
| --- | --- |
| Core | Server, proxy and bindings, request deduplication, logging, UI, startup, updates, health checks, global roots, streaming, model cache, and model pricing. Include Monitor retention and runtime limits where stored outside AppConfig. |
| Clients and access | Client lifecycle, connection templates and sync, LLM/MCP modes, roots, privacy and payload retention, credential policies, upstream/native tools, firewall rules, and per-client feature overrides. |
| Providers and routing | Provider types and instances, auth, privacy settings, health, managed models, pricing, strategies, routing profiles, named routes and versions, model permissions, free tier, rate limits, session/adaptive routing, and cache options supported by the existing config. |
| MCP and skills | External servers and transports, auth, sampling, elicitation, Push Relay, client webhook setup, marketplace, skill sources, coding agents, MCP via LLM, and tool indexing. |
| AI and request features | Guardrails, secret scanning, memory, context management, prompt compression, JSON repair, System One, Responses retention, local models, and usage tracking. |
| Application-owned state | Schema versions, migrations, generated IDs, observed capabilities, and cache artifacts. Expose useful status where appropriate. Treat account secrets, provider keys, client tokens, webhook signing keys, and credential-backed URLs as setup workflows. |

Review the manifest against the current Rust types and desktop Tauri commands. Fail the parity check on uncategorized fields. Paths with process starts, port binds, external app edits, credential changes, or provider registry updates use the same typed operations and notifications as the desktop. Design the preview/apply service to coordinate validation, persistence, and runtime reconciliation, including retryable partial failures and conflicting desktop edits. Return compact affected-object lists and sanitized errors.

## Implementation and verification

1. Use the existing virtual-server trait and gateway permission machinery. Wire the admin catalog into the desktop built-in list, client permission UI, and demo mock. Keep operational dependencies in shared services called by both MCP and Tauri, with the desktop adapter for approvals and browser interactions.
2. Start with a complete debug workflow across client creation, provider setup, credential handoff, provider check, model discovery, routing preview/apply, and firewall preview/apply. Build the rest of the manifest, operations, and guides onto the same contracts.
3. Use existing keychain/OAuth services for provider and client credentials. Return setup status or a short-lived, scoped setup handoff without plaintext secrets. Use URL-mode elicitation when supported and an explicit desktop action otherwise. Sanitize tool errors and audit fields. Use typed inputs, structured results with text fallback, bounded output, and MCP annotations once the gateway preserves them.
4. Extend the current intercept rule and pending approval flow. Keep inspect/control tool calls reachable while the rule applies. Match the actual intercepted request and expected pending state when responding. Preserve one-shot behavior and isolate concurrent desktop and MCP edits. Record actor, target, operation, and outcome in the existing Monitor/audit path with payload privacy applied.
5. Run stable Rust formatting, Clippy and workspace tests, plus frontend unit, TypeScript, desktop build, and website build checks as applicable. Add focused tests for wildcard denial, direct-call denial, revocation during an approval, invalid config and failed writes, strategy sharing, credential redaction, model pagination/provider refresh failures, privacy-filtered event reads, intercept expiry, duplicate responses, and desktop/MCP conflict handling.
6. Start `cargo tauri dev --no-watch` using the debug environment and verify the app and actual gateway port. Use disposable clients and a mock or local provider for end-to-end smoke tests. Verify the built-in row, unauthorized absence and denial, skill discovery, client/strategy side effects, live model list, Monitor events, and hold/deny/expire of a test request. Leave the debug app running and give Matus the UI path and a copyable test prompt.

## Research and repository references

- MCP defines discovery, input/output schemas, annotations, and initialization instructions. Use compact startup instructions and independently discoverable callable tools. [MCP tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools) · [MCP lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)
- Agent Skills recommends exposing basic metadata first and loading skill content and references on demand. [Agent Skills specification](https://agentskills.io/specification)
- Datadog's MCP offers toolsets, scoped observability data, filters, and audit history. This supports the proposed scoped metadata search, separate payload access, and audit trail. [Datadog tools](https://docs.datadoghq.com/mcp_server/tools/) · [Datadog MCP access control](https://docs.datadoghq.com/account_management/rbac/mcp_access/)
- MCP URL elicitation supports credential entry outside the assistant. [MCP TypeScript SDK elicitation](https://ts.sdk.modelcontextprotocol.io/v2/servers/elicitation)
- mitmproxy's inspect, resume, and discard workflow informs the rule and pending-request tool split. [mitmproxy interception](https://docs.mitmproxy.org/stable/mitmproxytutorial-interceptrequests/)
- Repository entry points include `crates/lr-mcp/src/gateway/virtual_server.rs`, `crates/lr-config/src/types.rs`, `src-tauri/src/ui/commands_clients.rs`, `commands_routing_profiles.rs`, `commands_providers.rs`, and `commands_monitor.rs`.

## Skill activation and intercept push

After the tool inventory review, the implementation also uses progressive tool loading. The first catalog contains the bootstrap tools. Calling `lr_skills_get` with `providers`, `model-routing`, or another skill enables its authorized tool group in that MCP session, invalidates the session's tool catalog, and sends `notifications/tools/list_changed` to that session. The response lists the unlocked tools and tells clients that ignore the notification to call `tools/list` again. A session starts with its own catalog. A loaded skill does not grant privileges and a guessed tool name is checked against both the loaded groups and current grants.

Use Push Relay for pending and resolved events when active-intercept monitoring is enabled by the operator. Negotiate the downstream client delivery mode through the relay and scope events to authorized client sessions. Event bodies carry identifiers, status and timing only. Clients without native push can read the held request list and use the relay's polling tools when available.

- [x] Verify bootstrap discovery, skill activation, notification delivery, session isolation, and direct call rejection before loading a skill on the dedicated endpoint.
- [x] Verify capability-aware push delivery, renewed scope checks, and polling fallback for intercept monitoring (see 2026-11-23-MCP_GATEWAY_FINAL.md, item 9).

## Debug preview

The preview implements all 61 tool names in the inventory. Some operations use a smaller initial scope than the proposed interface. Its configuration surface covers the main setup workflows and the settings sections reported by `lr_config_list_sections`. Provider/API credentials, OAuth account connection, and advanced transport setup use the desktop. Monitor exposes metadata and counters. The active intercept tool supports LLM and MCP requests, publishes pending and resolved metadata through Push Relay, and uses the existing desktop approval for release. Settings without a live runtime setter may require a restart.

The isolated preview is available at `http://127.0.0.1:33625/mcp/localrouter` with the disposable **Admin MCP Preview** and **Intercept Test Client** clients. The gateway smoke test verifies the dedicated endpoint, grant enforcement, independent sessions, skill activation, list-change delivery, the Push Relay event catalog, and a configuration dry run. The standard MCP SDK connects directly using Streamable HTTP and discovers the tools unlocked by the model-routing skill.

Local validation passed on rustup stable, including workspace tests, Clippy, and formatting. Desktop unit tests and builds passed. The website production build and static output checks passed.
