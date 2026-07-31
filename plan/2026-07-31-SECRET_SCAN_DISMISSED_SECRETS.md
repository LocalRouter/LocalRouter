# Permanently Ignoring Individual Secrets (without storing them)

## Problem

Secret scanning is all-or-nothing per client: a false positive (or an
intentionally-shared value like a local dev password) keeps producing a popup
on every request, and the only escapes are a 1-hour bypass or turning scanning
off for the client entirely. There is no way to say "this one value is fine,
keep scanning for everything else".

Storing the value to recognize it later is exactly what this feature must not
do — it would put a live credential in the config file in plaintext.

## Approach

Store a **salted PBKDF2-HMAC-SHA256 digest** of the value, never the value.

- **Salted** (random 16 bytes per client): defeats rainbow tables and stops the
  same secret being correlated across clients.
- **Slow** (600k iterations, OWASP's PBKDF2-HMAC-SHA256 guidance): detected
  secrets are not all high-entropy — many are human-chosen passwords — so a
  plain digest would fall to a dictionary attack if the config leaked.
- **Per-client salt, not per-entry**: keeps lookup to one KDF run instead of
  one per entry. The only leak versus per-entry salts is that two identical
  values in one client produce equal digests, which is how duplicates are
  deduped anyway.
- **Iterations stored per entry** so the cost can be raised later without
  invalidating existing entries.

Measured cost: ~110ms release / ~1.5s debug per KDF run. Too slow to repeat, and
a dismissed secret by definition recurs on every later request carrying it — so
verdicts are memoized per (client, secret) in `DismissalCache`, keyed by a
SHA-256 of the secret (never the secret), and invalidated whenever the client's
salt or entry set changes.

## Implementation

### Crypto + matching — `crates/lr-secret-scanner/src/dismissal.rs` (new)

`new_salt()`, `hash_secret()`, `verify_secret()` (constant-time, returns false
on malformed input so a corrupt entry can never read as "dismissed"), and
`DismissalCache`.

### Config — `crates/lr-config/src/types.rs`

`ClientSecretScanningConfig` gains `dismiss_salt: Option<String>` and
`dismissed_secrets: Vec<DismissedSecret>`, both `#[serde(default)]` for
backward compatibility. `DismissedSecret` holds id, hash, iterations, rule id
and description, a masked `hint` (the same mask already shown in popups and
recorded in monitor events), and an RFC3339 timestamp.

### Scan path — `crates/lr-server/src/routes/pipeline.rs`

`scan_request_for_secrets` filters dismissed findings out of the scan result
before deciding anything. If that empties the findings, the request is allowed
and the monitor event completes as `dismissed` rather than `pass`, so the
monitor still shows the scan happened and why nothing fired.

### Commands — `src-tauri/src/ui/commands_clients.rs`

- `ignore_secret_permanently(request_id, finding_index)` — reads the plaintext
  from the live approval session, hashes it on a blocking thread, appends the
  entry (deduped by digest), invalidates the cache.
- `list_client_dismissed_secrets(client_id)`
- `remove_client_dismissed_secret(client_id, entry_id?)` — one entry, or all.
- `update_client_secret_scanning_config` must preserve `dismiss_salt` and
  `dismissed_secrets`: it replaces the struct wholesale from a UI payload that
  only carries `action`, so without this, changing the action silently drops
  every exception.

### UI

- Popup: per-finding "Never flag this value again for {client}" button, with
  the tooltip stating a salted hash is stored and where to undo it. Once every
  finding is ignored there is nothing left to decide, so the popup resolves
  itself (allow for Ask, close for Notify).
- Per-client Secret Scanning tab: an "Ignored Values" card listing rule, masked
  hint and date, with per-entry removal and a confirmed "Reset All".

## Mandatory final steps

1. **Plan review** — re-read this plan against the implementation.
2. **Test coverage review** — cover new/changed paths.
3. **Bug hunt** — fresh-eyes pass over the diff.
4. **Commit**, push, cut a release.
