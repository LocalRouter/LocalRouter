- [x] Trace browser OAuth, legacy client-credentials OAuth, bearer tokens, discovery and custom-header storage.
- [x] Clarify OAuth choices: browser sign-in with optional registered-app settings versus client credentials without sign-in; existing access tokens use Bearer Token.
- [x] Make None the default and Custom headers a separate option in creation, marketplace and settings; only submit selected authentication credentials.
- [x] Use existing secure custom_headers auth storage; preserve saved secret references and legacy transport headers without silently changing existing configurations.
- [x] Keep None and manual authentication choices stable during discovery; only browser OAuth may trigger browser authentication.
- [x] Update command types/demo mocks and documentation as required; cover auth selection, credentials, header persistence/removal, legacy reload, discovery and no browser login for client credentials.
- [x] Plan Review: compare implementation and intended behavior.
- [x] Test Coverage Review: inspect all new/changed paths and error handling.
- [x] Bug Hunt: inspect credential preservation, conflicting headers, auto-save and form reset.
- [x] Validate exact stable Rust CI checks, TypeScript, production builds and relevant unit/browser regressions.
- [x] Commit and push completed validated work and automatic catalog updates on the existing isolated worktree branch.

Do not migrate legacy OAuth flows or discard non-auth transport headers from existing servers. Custom header values use keychain-backed auth config; blank existing values retain the matching saved header secret. Omitting a header removes it. Default None sends no credentials for newly created servers. Browser app credentials should be an optional expanded section, while Google/Slack templates continue requiring registered client credentials.

Validation: stable 1.99.1 workspace Clippy with warnings denied, formatting and full Rust suite passed; TypeScript, 100 unit tests, 20 MCP browser tests, desktop/website builds and 2 packaging regressions passed. Bug hunt covered last-header removal and preventing stale browser client-secret reuse after changing authentication modes.
