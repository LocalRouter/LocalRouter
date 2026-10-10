# Marketplace MCP installs: bearer token never found

## Problem

Both marketplace installers (`marketplace_install_mcp_server_direct` and the
marketplace service install callback in `main.rs`) stored the bearer token in
the keychain under the bare server id, with `token_ref: "pending"` in the
config. The MCP manager reads `{server_id}_bearer_token`, the account every
other path (MCP server UI, clone) uses, so a server installed from the
marketplace with a bearer token always connected without its token. The
file-to-keychain secret migration also skipped the bare-id secret because no
config entry referenced it.

## Fix

- `lr_config::mcp_bearer_token_account(server_id)` names the account; every
  writer and the reader use it.
- Marketplace configs record that account as `token_ref`.
- Existing installs (`token_ref: "pending"`, constant
  `LEGACY_PENDING_BEARER_TOKEN_REF`) keep working: the manager falls back to
  the bare-id account, cloning reads it, and the secrets-file migration keeps
  it referenced.

## Tests

- `install::tests::test_create_mcp_server_config_http_sse` asserts the
  `token_ref` is the account the manager reads.
- `storage::tests::legacy_pending_bearer_token_keeps_its_bare_id_secret`
  (secret references extracted into `referenced_secret_keys`).
- Workspace clippy `-D warnings`, fmt; tests of lr-config, lr-marketplace,
  lr-mcp (the app's MCP and marketplace command modules have no tests).

## Final steps

- [x] Plan review, test coverage review, bug hunt (deleting a legacy server
  leaves the bare-id secret behind, as before; harmless).
- [x] Commit and push.
