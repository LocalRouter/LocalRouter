# Add MCP server connection test

- [x] Share resolved creation settings with the connection test for custom, template, and marketplace servers.
- [x] Add an explicit Test connection action before Create, with pending, success, auth-required and failure feedback. Invalidate results when settings change. OAuth login remains after creation.
- [x] Plan Review: compare implementation with these requirements.
- [x] Test Coverage Review: verify resolved credentials, template inputs, and stale result behavior.
- [x] Bug Hunt: review asynchronous updates and authentication reporting.
- [x] Run frontend unit tests, desktop build and website build check.
- [x] Commit, merge with --no-ff into master, and push.

Validation: 146 frontend unit tests pass, including four new transport-resolution regressions. Desktop production build and website build:check pass (bundle-size and existing mixed-import warnings). A temporary browser harness exercised real component rendering with mocked Tauri responses: exact request parameters, pending state, success, edit invalidation, stale in-flight results, authentication required, OAuth guidance, errors, and empty targets. No Rust code changed; no Rust checks run. Connection tests use the existing bounded unsaved discovery command and never save a server or invoke tools. Test connection is optional; OAuth authentication remains after creation.

Plan review, coverage review and bug hunt completed. Shared resolution keeps test and creation settings consistent; component remounting discards results when settings change or the form closes.
