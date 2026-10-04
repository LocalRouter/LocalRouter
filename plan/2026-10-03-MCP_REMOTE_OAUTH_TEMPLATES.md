# Remote MCP browser login and service templates

## Todo
- [x] Inspect transport, OAuth, server forms, current plans, and release workflow.
- [x] Review latest MCP 2026-07-28 compatibility without adding gateway paths; preserve endpoint/query URLs and legacy session handling.
- [x] Expose browser OAuth for custom servers; discover metadata, automatically register public clients, persist credentials, request resource-bound tokens, and support login after creation and reauthentication.
- [x] Add Atlassian v2 and Datadog templates, including configurable Datadog toolsets and locally bundled official branding or the established generic MCP icon.
- [x] Synchronize modified commands, TypeScript contracts, and website mocks.
- [x] Plan review: compare implementation and requested behavior, and complete gaps.
- [x] Test coverage review: add meaningful transport, discovery, registration, and template tests.
- [x] Bug hunt: review login state, credential handling, URL construction, sessions, and failure recovery.
- [x] Validate frontend and stable Rust workspace (update stable, Clippy, fmt, tests).
- [x] Commit and push task changes and any automatic catalog updates, preserving unrelated work.
- [x] Hold release publication until the user explicitly resumes it (2026-10-03 steering).

## Implementation and review

`/v2/mcp` is Atlassian's service endpoint, not a protocol revision. Existing dual-era MCP support was checked against the official final 2026-07-28 changelog, versioning, Streamable HTTP, and subscription specifications. No inbound gateway paths were added.

- Remote transport: retain legacy initialize session IDs, accept POST-only servers, preserve JSON-RPC error bodies for modern era detection, read POST SSE notifications/results incrementally, and return final results without waiting for stream closure. Legacy servers with missing SSE content types remain supported.
- Modern subscriptions: use POST `subscriptions/listen` for upstream list changes; remove the ordinary-request deadline from long-lived streams. Downstream subscription filters use the final spec's `notifications` object, explicit opt-in, requested resource URIs, notification acknowledgment, and originating request ID. Modern result metadata includes gateway server identity; backend discovery reads the finalized identity location.
- Browser OAuth: custom/template/server-settings forms expose login. Discovery strips toolset query parameters and supports path/root metadata and advertised challenge URLs. Public-client registration includes `application_type: native`, PKCE-compatible credentials and callback URI. Registered client IDs persist; issuer changes re-register instead of replaying old credentials. Tokens use the canonical endpoint resource parameter and keychain storage; reconnects refresh expired tokens with serialized refresh to preserve rotating refresh tokens.
- Creation carries pending login through the list/detail panel remount. Login success refreshes server config/auth/health. Cancellation ignores delayed flow results. Manual registered-client settings remain available.
- Templates use the exact requested Atlassian v2 and Datadog endpoints. Datadog offers officially documented toolsets, including DDSQL and explicit preview toolsets; URL previews and persisted query values agree. Official logos are bundled locally with source/trademark records.
- Tauri response contracts and demo mocks now match the browser-flow, discovery, and persisted auth structures.

## Validation

- Frontend production build, app/website TypeScript, 31 frontend unit tests, and all 3 MCP browser integration tests pass. The browser tests verify Atlassian's URL/login, persisted DDSQL query selection and automatic login after navigation, and custom HTTP browser auth without required manual credentials.
- MCP unit/transport regressions cover nested discovery paths, query stripping, root fallback, native client registration/reuse/issuer changes, registration errors, expired tokens, resource-bound and serialized public-client refresh, legacy session headers, still-open SSE responses and notifications, modern error bodies, and POST subscriptions.
- Stable rustup updated to rustc 1.99.0. Stable workspace Clippy with warnings denied and formatting pass. Full workspace tests and documentation tests pass. Logs: `/private/tmp/mcp-clippy-verified.log`, `/private/tmp/mcp-workspace-tests-verified.log`, `/private/tmp/mcp-frontend-build.log`, and `/private/tmp/mcp-browser-tests-final.log`.
- Browser tests use local demo credentials; no real provider account was authorized. Public metadata was inspected for both services.
- Automatic models.dev refresh was included by concurrent commit `78ce2274`; there is no remaining catalog delta to stage.
- Preserve the pre-existing reverse-proxy/startup changes and their plan. Release remains on hold.

Sources: https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/changelog.mdx ; https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/patterns/subscriptions.mdx ; https://atlassian.github.io/atlassian-mcp-server/ ; https://docs.datadoghq.com/mcp_server/setup/ .
