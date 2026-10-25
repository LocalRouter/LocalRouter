- [x] Inspect official Google requirements and existing template/OAuth flow.
- [x] Create an isolated worktree for implementation.
- [x] Add official Gmail and Drive templates with registered OAuth credentials and setup guidance.
- [x] Prepare a Google Cloud test project in the browser for matus@smotana.com; report preview or consent blockers.
- [x] Document per-user project requirements; no shared LocalRouter OAuth project or bundled credentials.
- [x] Plan Review: compare implementation with all requested behavior.
- [x] Test Coverage Review: verify template creation/authentication behavior and uncovered paths.
- [x] Bug Hunt: review scopes, redirects, credentials and backward compatibility.
- [x] Run TypeScript and exact stable Rust CI checks.
- [x] Commit and push task changes and the automatic catalog snapshot update.

Keep the user's production LocalRouter configuration untouched; the user will add servers in the UI. Use existing browser OAuth, Streamable HTTP transport and bundled official logos. Investigate Google refresh-token requirements; document any installed-app limitations without claiming unverified connectivity. Never commit test credentials.


## Implementation and review

Added official remote Gmail and Google Drive templates with required personal OAuth client credentials, per-service documented scopes, multiline setup instructions and official setup links. Gmail uses a bundled official Google asset; Drive reuses its existing bundled asset. The generic browser-auth creation helper now accepts template default scopes, preserving saved scopes on later edits. Google browser authorization requests offline access and renewed consent for refresh tokens, while explicit provider overrides remain authoritative. Other authorization hosts are unchanged.

Plan review verified both templates use HTTP and browser OAuth, no shared client credentials, and no production configuration writes. Coverage includes both complete template creation flows, required credentials, empty custom headers, exact endpoint/scopes and browser-flow start; saved-scope preservation and Google refresh parameters/overrides/lookalike hosts are covered separately. Bug review checked issuer preparation preserves explicit scopes, callback URI parity, secret fields and existing auth behavior.

Personal browser setup: created LocalRouter MCP Test (`erudite-hope-512700-s9`) under matus@smotana.com in smotana.com. Google confirmed successful enablement of Gmail API, Gmail MCP API, Google Drive API and Drive MCP API. OAuth branding form is prepared with Internal audience and personal support/contact address; stopped before accepting Google's API Services User Data Policy, with action-time confirmation requested. No OAuth client credentials or user data grants created yet; no live authenticated tool invocation claimed.

Validation: stable 1.99.1 updated and actual compiler path pinned in PATH/RUSTC; stable workspace Clippy with warnings denied and formatting check passed. Desktop and website production builds passed; 96 unit tests and both new template browser regressions passed. All 15 remote MCP browser regressions passed. Full Rust workspace tests passed: 4,598 passed, 69 ignored, zero failures across 111 suites. Automatic models.dev snapshot refresh will be included as required. No release/deploy is part of this task.


Personal setup completed after user approval: OAuth app and personal agent client were created with the localhost callback. User was given the credentials directly; they are not recorded in the repository. Subsequent authorized audience change to External/Testing and all three requested test accounts is recorded in the Google MCP creation-error follow-up plan. Initial implementation was committed and pushed as e3d0552e on feat/google-workspace-mcp.
