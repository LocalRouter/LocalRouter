- [x] Record user authorization to include all recent committed master changes and publish a stable release.
- [x] Finish authentication fixes and validate the worktree.
- [x] Commit auth changes, merge latest local/remote master into the isolated worktree and resolve any conflicts while preserving unrelated work.
- [x] Review the full release diff since the latest published tag, including all recently committed changes.
- [x] Plan Review: verify the auth behavior, headers, Google templates/issuer fix, popup errors, icons and negotiated protocol changes are included.
- [x] Test Coverage Review: verify combined-tree regressions and backward compatibility.
- [x] Bug Hunt: inspect merge resolution, credentials, secret references and release inputs.
- [x] Run exact stable Rust CI checks, frontend/website builds and relevant unit/browser/packaging tests on the combined tree.
- [ ] Commit and push combined validated changes to master with a normal push; retain uncommitted files in the user's main checkout.
- [ ] Dispatch the existing Release workflow with the next unused stable version (workflow owns bump/tag).
- [ ] Monitor candidate CI and release builds, fix any actual failures and verify public release assets, hashes, updater signatures and version metadata.
- [ ] Report release publication and distinguish any asynchronous packaging follow-ups.

Use the dedicated Rust build directory /private/tmp/localrouter-google-mcp-target because shared cache artifacts were removed by another build during validation. Stable compiler is pinned by PATH/RUSTC. Local dev/test debug symbols and incremental artifacts are disabled to conserve disk; cargo commands/targets/warnings match required CI checks.

Keep the user's Google OAuth project, credentials and account authorization local; never publish credentials. No additional Google account authorization or app configuration changes are required for this release.

Combined release content reviewed: Google templates and personal-project setup instructions, narrow Google issuer alias fix, in-dialog errors, explicit auth choices and secure header updates, negotiated HTTP protocol version, and resource favicon discovery. Merge completed without conflicts and preserved all existing resource-icon hooks/types/mocks.

Validation: stable 1.99.1 workspace Clippy with warnings denied, formatting and full Rust tests passed. TypeScript, 105 unit tests, 20 MCP browser regressions, desktop and website production builds, and 2 packaging regressions passed on the merged tree.
