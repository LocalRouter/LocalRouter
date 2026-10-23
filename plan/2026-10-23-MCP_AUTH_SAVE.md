- [x] Trace authentication persistence and current save behavior.
- [x] Keep debounced auto-save, add visible saving/saved/error feedback and validation; serialize edits made during an in-flight save.
- [x] Persist pre-generated OAuth credentials and preserve existing secret/metadata on partial edits; cover switching authentication methods.
- [x] Accept frontend OAuth spelling without changing legacy serialized configuration; add the official Slack browser OAuth template with client ID and secret.
- [x] Move OAuth Authentication below Server Configuration.
- [x] Plan Review: compare implementation with intended save behavior.
- [x] Test Coverage Review: cover authentication transitions, credential preservation and validation failures.
- [x] Bug Hunt: review async saves, credential handling and form initialization.
- [x] Validate TypeScript and exact stable Rust CI checks, then commit and push task changes and automatic catalog updates.
- [ ] Dispatch the next unused stable release through the existing Release workflow; verify publication, updater assets and signatures.

## Findings and implementation

Rust serialized OAuth as `o_auth` while the UI only recognized `oauth`. Loading Settings therefore selected None and attempted to clear auth with JSON null, which the backend rejected as an enum. Creation also sent the unrecognized `oauth` spelling, and the edit handler never saved pre-generated credentials or watched either credential field.

The backend accepts both spellings while preserving legacy `o_auth` serialization, accepts null/explicit None clearing, and retains existing secret references when credentials are left blank. The UI recognizes both spellings, watches all credential fields, validates incomplete edits, displays auto-save status/errors, and serializes/coalesces saves. Form value snapshots replace timer-based suppression, including immediate edits and returns to a previous method/value. No Save button was added. Registered app credentials are also editable with browser OAuth. OAuth Authentication now follows Server Configuration.

Slack’s template uses the existing bundled Slack logo, official endpoint and a required Client ID/Client Secret form. It stores the supplied app credentials through the existing keychain path and starts browser authorization with that registered app. Source: https://docs.slack.dev/ai/slack-mcp-server/ . Pre-generated client-credentials OAuth remains a separate, backward-compatible choice; no existing servers are migrated between flows.

Plan/coverage/bug review: checked secret preservation, empty/new credentials, null versus omitted auth, both OAuth spellings, discovery errors, immediate edits after navigation, no refresh-triggered save loop, overlapping edits and reverting in-flight edits. Removed raw auth update logging. The demo mirrors canonical backend metadata and does not retain submitted secrets.

Validation so far: rustup stable 1.99.1 updated/verified; actual compiler pinned by PATH and RUSTC, inaccessible sccache disabled only for validation. Workspace Clippy with warnings denied and formatting passed. Full Rust suite: 4,597 passed, 69 ignored, zero failures across 111 suites. All 95 frontend unit tests and packaging regressions passed; desktop and website production builds passed. Browser OAuth regressions use an isolated headless Chrome demo session. GitHub CLI TLS failed through the inspection proxy; release API calls use macOS curl with normal certificate verification and existing GitHub authentication, without exposing credentials in argv/output.

Final validation: all 12 remote MCP browser regressions passed; an additional in-flight credential-reversion regression passed (13 total). Final TypeScript and production frontend/website builds passed after the snapshot guard changes. Upstream is unchanged; v0.0.167 is the next unused stable version.
